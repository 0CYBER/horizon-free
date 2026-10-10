//! The camera type (`CameraType`: first person, third person behind and
//! in front, cycled by F5), the detached camera (`Camera.setup` with
//! `getMaxZoom`), and the local player drawn as a model: in third person
//! and in the inventory (`InventoryScreen.extractEntityInInventoryFollowsMouse`).
use super::{Game, Screen};
use horizon_client::{
    hud::HudBatch,
    mobs::{Anim, Brain, Mob, MobType},
    pose::Pose,
};
use horizon_core::geometry::Vec3;
use std::{path::Path, sync::Arc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum CameraType {
    #[default]
    FirstPerson,
    ThirdPersonBack,
    ThirdPersonFront,
}
impl CameraType {
    pub(super) fn first_person(self) -> bool {
        self == CameraType::FirstPerson
    }
    fn cycle(self) -> Self {
        match self {
            CameraType::FirstPerson => CameraType::ThirdPersonBack,
            CameraType::ThirdPersonBack => CameraType::ThirdPersonFront,
            CameraType::ThirdPersonFront => CameraType::FirstPerson,
        }
    }
}
/// The player's model type (`PlayerModel` with the default skin).
pub(super) fn player_type(root: &Path) -> Option<Arc<MobType>> {
    horizon_client::mobs::load_model(root, "minecraft:player#main").map(|model| {
        Arc::new(MobType {
            id: "minecraft:player".into(),
            // Players render as a humanoid; the species only drives local AI.
            species: horizon_client::mobs::Species::Zombie,
            width: 0.6,
            height: 1.8,
            eye_height: 1.62,
            category: "misc".into(),
            max_health: 20.0,
            speed: 0.1,
            flying_speed: 0.02,
            attack_damage: 1.0,
            attack_knockback: 0.0,
            follow_range: 0.0,
            step_height: 0.6,
            armor: 0.0,
            armor_toughness: 0.0,
            knockback_resistance: 0.0,
            jump_strength: 0.42,
            gravity: 0.08,
            safe_fall_distance: 3.0,
            fall_damage_multiplier: 1.0,
            water_efficiency: 0.0,
            water_slowdown: 0.8,
            air_speed: 0.02,
            baby_scale: 0.5,
            head_rot_speed: 10.0,
            max_head_x_rot: 40.0,
            max_head_y_rot: 75.0,
            ambient_interval: 80,
            flags: Default::default(),
            model: Arc::new(model),
            layers: Default::default(),
            texture: "entity/player/wide/steve".into(),
            brain: Brain::Passive,
            anim: Anim::Humanoid { zombie_arms: false },
            sounds: [
                "entity.player.hurt",
                "entity.player.hurt",
                "entity.player.death",
            ],
            step_sound: None,
            food: Arc::from([]),
            tempt: Arc::from([]),
        })
    })
}
fn wrap(d: f32) -> f32 {
    let d = d % 360.0;
    if d >= 180.0 {
        d - 360.0
    } else if d < -180.0 {
        d + 360.0
    } else {
        d
    }
}
impl Game<'_> {
    /// `key.togglePerspective`.
    pub(super) fn cycle_camera(&mut self) {
        self.camera_type = self.camera_type.cycle();
    }
    /// `Camera.setup`: the eye, or a point up to four blocks behind (or in
    /// front of) it, pulled in where blocks are in the way. Returns the
    /// position, yaw and pitch to render from.
    pub(super) fn camera_view(&self, eye: glam::DVec3) -> (glam::DVec3, f32, f32) {
        let (mut yaw, mut pitch) = (self.yaw, self.pitch);
        if self.camera_type.first_person() || self.player.is_none() || self.sleeping.is_some() {
            return (eye, yaw, pitch);
        }
        if self.camera_type == CameraType::ThirdPersonFront {
            yaw += 180.0;
            pitch = -pitch;
        }
        let f = super::view_vector(yaw, pitch);
        let mut dist = 4.0f64;
        for i in 0..8 {
            let o = [
                f64::from((i & 1) * 2 - 1) * 0.1,
                f64::from((i >> 1 & 1) * 2 - 1) * 0.1,
                f64::from((i >> 2 & 1) * 2 - 1) * 0.1,
            ];
            let from = [eye.x + o[0], eye.y + o[1], eye.z + o[2]];
            let back = [-f[0], -f[1], -f[2]];
            if let Some(hit) = self.clip_ray(from, back, dist) {
                let d = glam::DVec3::from_array(hit.point) - eye;
                dist = dist.min(d.length());
            }
        }
        (eye - glam::DVec3::from_array(f) * dist, yaw, pitch)
    }
    /// Keep the player's model in step with the player (`LivingEntity.tick`
    /// body turning, `WalkAnimationState`, the swing and the pose).
    pub(super) fn tick_player_model(&mut self) {
        let Some(p) = self.player.as_ref() else {
            self.player_model = None;
            return;
        };
        if self.player_model.is_none() {
            let Some(kind) = self.res.player_type.clone() else {
                return;
            };
            self.player_model = Some(Mob::new(0, kind, p.position, self.yaw));
        }
        let crouching = p.pose == Pose::Crouching;
        let (position, flying) = (p.position, p.flying || p.fall_flying);
        let attack = self.swing_anim;
        let swinging = self.swing.is_some();
        let (yaw, pitch) = (self.yaw, self.pitch);
        let main = self.inventory[self.selected]
            .as_ref()
            .map(|s| s.item.clone());
        let off = self.offhand.as_ref().map(|s| s.item.clone());
        let riding = self.riding.is_some();
        let hurt = p.health.hurt.hurt_time;
        let Some(m) = self.player_model.as_mut() else {
            return;
        };
        let at = Vec3::new(
            position.x,
            position.y - if crouching { 0.125 } else { 0.0 },
            position.z,
        );
        let (dx, dy, dz) = (
            at.x - m.position.x,
            at.y - m.position.y,
            at.z - m.position.z,
        );
        m.previous = m.position;
        m.position = at;
        if dx * dx + dy * dy + dz * dz > 100.0 {
            m.previous = at;
        }
        m.previous_yaw = m.yaw;
        m.previous_body_yaw = m.body_yaw;
        m.previous_head_yaw = m.head_yaw;
        m.previous_head_pitch = m.head_pitch;
        m.yaw = yaw;
        m.head_yaw = yaw;
        m.head_pitch = pitch;
        let mut target = m.body_yaw;
        if dx * dx + dz * dz > 0.002_500_000_2 {
            target = (dz.atan2(dx) as f32).to_degrees() - 90.0;
        }
        if swinging {
            target = yaw;
        }
        // tickHeadTurn: ease toward the target, within 50 degrees of the head.
        m.body_yaw += wrap(target - m.body_yaw) * 0.3;
        let diff = wrap(yaw - m.body_yaw);
        if diff.abs() > 50.0 {
            m.body_yaw += diff - diff.signum() * 50.0;
        }
        // WalkAnimationState.update(min(distance * 4, 1), 0.4).
        let dist = (dx * dx + dz * dz + if flying { dy * dy } else { 0.0 }).sqrt() as f32;
        m.previous_limb_speed = m.limb_speed;
        m.limb_speed += ((dist * 4.0).min(1.0) - m.limb_speed) * 0.4;
        m.limb_position += m.limb_speed;
        m.age = m.age.wrapping_add(1);
        m.anim.attack_anim_o = attack.0;
        m.anim.attack_anim = attack.1;
        m.extra.main_hand = main;
        m.extra.off_hand = off;
        m.extra.crouching = crouching;
        m.extra.passenger = riding;
        m.hurt_time = hurt;
    }
    /// Whether the player's own model shows in the world view.
    fn player_model_shown(&self) -> bool {
        !self.camera_type.first_person()
            && self.sleeping.is_none()
            && !self.spectator()
            && self.potions.effects.get("minecraft:invisibility").is_none()
            && self
                .player
                .as_ref()
                .is_some_and(|p| !p.health.dead_or_dying())
    }
    /// The player's model for the entity pass in third person.
    pub(super) fn player_model_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<horizon_client::mesher::Vertex>,
    ) {
        if !self.player_model_shown() {
            return;
        }
        let Some(m) = self.player_model.as_ref() else {
            return;
        };
        let light = self.world.light(
            m.position.x.floor() as i32,
            (m.position.y + 1.62).floor() as i32,
            m.position.z.floor() as i32,
        );
        let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
        let atlas = &self.res.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        horizon_client::mobs::render::render_with(
            m,
            alpha as f32,
            camera,
            &lookup,
            packed,
            &self.res.mob_data,
            &self.player_armor_layers(),
            out,
        );
        self.held_item_geometry(m, camera, alpha, packed, true, out);
    }
    /// `extractEntityInInventoryFollowsMouse`: the player in the box
    /// `[x0, x1) x [y0, y1)`, `size` pixels per block, turned toward the
    /// cursor; drawn as depth-sorted, back-face-culled quads.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_player_preview(
        &self,
        batch: &mut HudBatch,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        size: f32,
        offset_y: f32,
    ) {
        let Some(m) = self.player_model.as_ref() else {
            return;
        };
        if self
            .player
            .as_ref()
            .is_some_and(|p| p.health.dead_or_dying())
        {
            return;
        }
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let x_angle = ((cx - self.cursor.0) / 40.0).atan();
        let y_angle = ((cy - self.cursor.1) / 40.0).atan();
        let mut m = m.clone();
        // The render state: still, facing out, the head toward the cursor.
        m.previous = m.position;
        m.body_yaw = 180.0 + x_angle * 20.0;
        m.previous_body_yaw = m.body_yaw;
        m.head_yaw = m.body_yaw + x_angle * 20.0;
        m.previous_head_yaw = m.head_yaw;
        m.head_pitch = -y_angle * 20.0;
        m.previous_head_pitch = m.head_pitch;
        m.position.y += 0.125 * f64::from(u8::from(m.extra.crouching));
        m.previous = m.position;
        let camera = glam::DVec3::new(m.position.x, m.position.y, m.position.z);
        let atlas = &self.res.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        let mut verts = Vec::new();
        horizon_client::mobs::render::render_with(
            &m,
            1.0,
            camera,
            &lookup,
            240 | 240 << 8,
            &self.res.mob_data,
            &self.player_armor_layers(),
            &mut verts,
        );
        self.held_item_geometry(&m, camera, 1.0, 240 | 240 << 8, false, &mut verts);
        let height = if m.extra.crouching { 1.5 } else { 1.8 };
        // PictureInPictureRenderer: the box centre, (size, size, -size);
        // then the translation and Rz(pi) * Rx(y_angle * 20 degrees).
        let gui = glam::Mat4::from_translation(glam::Vec3::new(cx, cy, 0.0))
            * glam::Mat4::from_scale(glam::Vec3::new(size, size, -size))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, height / 2.0 + offset_y, 0.0))
            * glam::Mat4::from_rotation_z(std::f32::consts::PI)
            * glam::Mat4::from_rotation_x((y_angle * 20.0).to_radians());
        let mut quads: Vec<(f32, [[f32; 2]; 4], [[f32; 2]; 4], u32)> = Vec::new();
        for q in verts.as_chunks::<4>().0 {
            let p: [glam::Vec3; 4] =
                std::array::from_fn(|i| gui.transform_point3(glam::Vec3::from_array(q[i].pos)));
            let n = (p[1] - p[0]).cross(p[2] - p[0]);
            // The winding gives inward normals: faces with n.z >= 0 point away.
            if n.z >= 0.0 {
                continue;
            }
            let n = n.normalize_or_zero();
            // Lighting.Entry.ENTITY_IN_UI, roughly: lit from the front and above.
            let shade = (0.55 + 0.45 * -n.z - 0.15 * n.y).clamp(0.4, 1.0);
            let c = q[0].color;
            let ch = |s: u32| ((((c >> s) & 255) as f32 * shade) as u32) << s;
            let color = (c & 0xFF00_0000) | ch(0) | ch(8) | ch(16);
            let depth = p.iter().map(|v| v.z).sum::<f32>() / 4.0;
            quads.push((
                depth,
                p.map(|v| [v.x, v.y]),
                std::array::from_fn(|i| q[i].uv),
                color,
            ));
        }
        // Far (low z) first: the viewer is on the +z side.
        quads.sort_by(|a, b| a.0.total_cmp(&b.0));
        let start = batch.vertices.len();
        for (_, p, uv, color) in quads {
            batch.quad(p, uv, color);
        }
        batch.clip_since(start, x0, y0, x1, y1);
    }
    /// The survival inventory's player box (`InventoryScreen`).
    pub(super) fn draw_inventory_player(&self, batch: &mut HudBatch, ox: f32, oy: f32) {
        if self.screen == Screen::Inventory {
            self.draw_player_preview(
                batch,
                ox + 26.0,
                oy + 8.0,
                ox + 75.0,
                oy + 78.0,
                30.0,
                0.0625,
            );
        }
    }
}
