//! Leads on the game side: putting them on and taking them off mobs
//! (`Entity.interact`'s leash part), fence knots (`LeadItem.useOn` and
//! `LeashFenceKnotEntity.interact`), loading and saving who holds what,
//! breaking, and drawing the rope (`LeashFeatureRenderer`) and the knot.
use super::Game;
use horizon_client::{
    items::ItemStack,
    mesher::Vertex,
    mobs::leash::{self, Holder, Pending},
};
use horizon_world::nbt::Tag;

/// `Vec3.yRot(angle)`.
fn rot_y(v: glam::DVec3, angle: f64) -> glam::DVec3 {
    let (s, c) = (angle.sin(), angle.cos());
    glam::DVec3::new(v.x * c + v.z * s, v.y, v.z * c - v.x * s)
}
fn pack_light(light: u8) -> u32 {
    (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8)
}
fn abgr(r: f32, g: f32, b: f32) -> u32 {
    let c = |v: f32| (v * 255.0) as u32 & 255;
    0xFF00_0000 | c(b) << 16 | c(g) << 8 | c(r)
}
impl Game<'_> {
    fn lead_sound(&mut self, sound: &str, at: [f64; 3]) {
        self.audio
            .play(&format!("minecraft:item.lead.{sound}"), Some(at), 1.0, 1.0);
    }
    /// `Leashable.canHaveALeashAttachedTo` for the mob at `index` and a
    /// holder whose box centre is `center`: within snapping range and
    /// leashable.
    fn can_attach_lead(&self, index: usize, center: glam::DVec3) -> bool {
        let m = &self.mobs[index];
        if !leash::can_be_leashed(m) || !m.alive() {
            return false;
        }
        let b = m.bounds();
        let (lo, hi) = (b.min_position(), b.max_position());
        let mine = glam::DVec3::new(
            (lo.x + hi.x) / 2.0,
            (lo.y + hi.y) / 2.0,
            (lo.z + hi.z) / 2.0,
        );
        mine.distance(center) <= leash::SNAP
    }
    fn player_center(&self) -> glam::DVec3 {
        self.player.as_ref().map_or(glam::DVec3::ZERO, |p| {
            glam::DVec3::new(
                p.position.x,
                p.position.y + f64::from(p.dimensions.height) / 2.0,
                p.position.z,
            )
        })
    }
    /// The mobs on the player's lead within 32 blocks of a point.
    fn player_leashed(&self, around: glam::DVec3) -> Vec<usize> {
        (0..self.mobs.len())
            .filter(|&i| {
                let m = &self.mobs[i];
                m.extra.leash == Some(Holder::Player)
                    && (m.position.x - around.x).abs() <= 16.0
                    && (m.position.y - around.y).abs() <= 16.0
                    && (m.position.z - around.z).abs() <= 16.0
            })
            .collect()
    }
    /// `Entity.spawnAtLocation(level, Items.LEAD)`: the item entity draws its
    /// motion from its own random, not the level's.
    fn drop_lead_item(&mut self, at: [f64; 3]) {
        let v = super::dispenser::new_item_motion();
        self.spawn_item(ItemStack::new("minecraft:lead", 1), at, v, 10);
    }
    /// `Leashable.dropLeash` / `removeLeash`: the lead falls as an item.
    pub(super) fn break_leash(&mut self, id: u32, drop: bool, sound: bool) {
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        if horizon_client::mobs::leash::remove(m).is_none() {
            return;
        }
        // Leashable.tickLeash gates a dying entity's lead drop on entity_drops.
        let drop = drop && (m.alive() || self.ui.rules.bool("entity_drops"));
        let at = [m.position.x, m.position.y, m.position.z];
        if sound {
            self.lead_sound("break", at);
        }
        if drop {
            self.drop_lead_item(at);
        }
    }
    /// Holders read from a save become real once the entity is found; a
    /// holder that never shows up drops the lead after 100 ticks.
    pub(super) fn tick_leads(&mut self) {
        let player_uuid = self.level.player_uuid.unwrap_or([0, 0, 0, 1]);
        for i in 0..self.mobs.len() {
            let Some(pending) = self.mobs[i].extra.leash_pending else {
                continue;
            };
            let holder = match pending {
                Pending::Knot(p) => Some(Holder::Knot(p)),
                Pending::Uuid(u) if u == player_uuid => Some(Holder::Player),
                Pending::Uuid(u) => self.mob_by_uuid(u).map(Holder::Mob),
            };
            let m = &mut self.mobs[i];
            if let Some(h) = holder {
                if h == Holder::Player {
                    m.extra.leash_player = Some(player_uuid);
                }
                m.extra.leash = Some(h);
                m.extra.leash_pending = None;
            } else if m.age > 100 {
                m.extra.leash_pending = None;
                let at = [m.position.x, m.position.y, m.position.z];
                self.drop_lead_item(at);
            }
        }
    }
    /// The `leash` tag of a mob about to be saved.
    pub(super) fn write_leash(&mut self, tag: &mut Tag, index: usize) {
        let Tag::Compound(map) = tag else { return };
        map.remove(&"leash".into());
        let m = &self.mobs[index];
        let holder = m.extra.leash;
        let pending = m.extra.leash_pending;
        let uuid = match (holder, pending) {
            (Some(Holder::Player), _) => Some(self.level.player_uuid.unwrap_or([0, 0, 0, 1])),
            (Some(Holder::Mob(id)), _) => Some(self.mob_uuid(id)),
            (_, Some(Pending::Uuid(u))) => Some(u),
            _ => None,
        };
        let value = match (holder, pending) {
            (Some(h), _) => leash::to_nbt(h, uuid),
            (None, Some(Pending::Knot(p))) => leash::to_nbt(Holder::Knot(p), None),
            (None, Some(Pending::Uuid(u))) => leash::to_nbt(Holder::Player, Some(u)),
            _ => None,
        };
        if let Some(v) = value {
            map.insert("leash".into(), v);
        }
    }
    /// `Entity.interact`'s leash part for a mob. Returns whether it was used.
    pub(super) fn interact_leash(&mut self, index: usize, item: &str) -> bool {
        let sneaking = self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft);
        let id = self.mobs[index].id;
        let at = self.mobs[index].center();
        if !self.mobs[index].alive() {
            return false;
        }
        // Sneaking with mobs in hand: tie them to this one.
        if sneaking && leash::can_be_leashed(&self.mobs[index]) && !self.mobs[index].is_baby() {
            let b = self.mobs[index].bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            let center = glam::DVec3::new(
                (lo.x + hi.x) / 2.0,
                (lo.y + hi.y) / 2.0,
                (lo.z + hi.z) / 2.0,
            );
            let held = self.player_leashed(center);
            let mut any = false;
            for j in held {
                if j != index && self.can_attach_lead(j, center) {
                    self.mobs[j].extra.leash = Some(Holder::Mob(id));
                    any = true;
                }
            }
            if any {
                self.lead_sound("tied", at);
                return true;
            }
        }
        if item == "minecraft:shears" {
            let mut dropped = false;
            if self.mobs[index].extra.leash.is_some() {
                self.break_leash(id, true, false);
                dropped = true;
            }
            let others: Vec<u32> = self
                .mobs
                .iter()
                .filter(|m| m.extra.leash == Some(Holder::Mob(id)))
                .map(|m| m.id)
                .collect();
            for o in others {
                self.break_leash(o, true, false);
                dropped = true;
            }
            if dropped {
                self.audio
                    .play("minecraft:item.shears.snip", Some(at), 1.0, 1.0);
                self.damage_held_tool();
                return true;
            }
        }
        if self.mobs[index].extra.leash == Some(Holder::Player) {
            let creative = self.creative;
            self.break_leash(id, !creative, false);
            self.lead_sound("untied", at);
            return true;
        }
        if item == "minecraft:lead" {
            let center = self.player_center();
            if self.can_attach_lead(index, center) {
                if self.mobs[index].extra.leash.is_some() {
                    self.break_leash(id, true, false);
                }
                let holder = self.acting_player_uuid();
                self.mobs[index].extra.leash = Some(Holder::Player);
                self.mobs[index].extra.leash_player = Some(holder);
                self.lead_sound("tied", at);
                // The lead is always used up.
                if let Some(stack) = self.held_mut().as_mut() {
                    stack.count -= 1;
                    if stack.count == 0 {
                        *self.held_mut() = None;
                    }
                }
                return true;
            }
        }
        false
    }
    /// `LeadItem.useOn` and `LeashFenceKnotEntity.interact` on a fence.
    pub(super) fn interact_knot(&mut self) -> bool {
        let Some(hit) = self.pick() else {
            return false;
        };
        let name = self
            .res
            .facts()
            .get(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]));
        if !self
            .res
            .items()
            .block_tags
            .get("minecraft:fences")
            .is_some_and(|s| s.contains(&*name.name))
        {
            return false;
        }
        let knot_pos = hit.pos;
        let knot = leash::knot_state(knot_pos).center;
        let center = glam::DVec3::new(knot.x, knot.y, knot.z);
        let at = [center.x, center.y, center.z];
        let existing: Vec<usize> = (0..self.mobs.len())
            .filter(|&i| self.mobs[i].extra.leash == Some(Holder::Knot(knot_pos)))
            .collect();
        let held_item = self.held().map(|s| s.item.to_string());
        let sneaking = self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft);
        // Shears snip every lead on the knot.
        if held_item.as_deref() == Some("minecraft:shears") && !existing.is_empty() {
            for i in existing {
                let id = self.mobs[i].id;
                self.break_leash(id, true, false);
            }
            self.audio
                .play("minecraft:item.shears.snip", Some(at), 1.0, 1.0);
            self.damage_held_tool();
            return true;
        }
        let in_hand = self.player_leashed(center);
        // The knot is only made for the lead item, or when mobs already hang on it.
        if held_item.as_deref() != Some("minecraft:lead")
            && existing.is_empty()
            && in_hand.is_empty()
        {
            return false;
        }
        let mut attached = false;
        for j in in_hand {
            if self.can_attach_lead(j, center) {
                self.mobs[j].extra.leash = Some(Holder::Knot(knot_pos));
                attached = true;
            }
        }
        if !attached && !sneaking {
            let player = self.player_center();
            for i in existing {
                if self.can_attach_lead(i, player) {
                    self.mobs[i].extra.leash = Some(Holder::Player);
                    attached = true;
                }
            }
        }
        if attached {
            self.lead_sound("tied", at);
            self.swing_arm();
        }
        attached
    }
    /// `LeashFeatureRenderer` for every lead and `LeashKnotRenderer` for the knots.
    pub(super) fn leash_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let atlas = &self.gui.atlas;
        let white = atlas.font.white();
        let uv = [
            [white.u0, white.v1],
            [white.u1, white.v1],
            [white.u1, white.v0],
            [white.u0, white.v0],
        ];
        let lerp3 = |a: horizon_core::geometry::Vec3, b: horizon_core::geometry::Vec3| {
            glam::DVec3::new(
                a.x + (b.x - a.x) * alpha,
                a.y + (b.y - a.y) * alpha,
                a.z + (b.z - a.z) * alpha,
            )
        };
        let mut knots: Vec<[i32; 3]> = Vec::new();
        for m in &self.mobs {
            let Some(holder) = m.extra.leash else {
                continue;
            };
            let body = f64::from(
                m.previous_body_yaw
                    + horizon_client::mobs::living::wrap(m.body_yaw - m.previous_body_yaw)
                        * alpha as f32,
            )
            .to_radians();
            // EntityRenderer.extractRenderState: one lead from the mob's leash
            // offset to the holder's rope hold position, or, when the holder
            // is a happy ghast and the mob supports a quad lead, four from
            // `getQuadLeashOffsets` to `getQuadLeashHolderOffsets`.
            let mut ends: Vec<(glam::DVec3, glam::DVec3, bool)> = Vec::new();
            let quad = match holder {
                Holder::Mob(id) if leash::supports_quad_leash(m.species()) => self
                    .mobs
                    .iter()
                    .find(|o| o.id == id)
                    .and_then(|o| leash::quad_holder_offsets(o).map(|h| (o, h))),
                _ => None,
            };
            if let Some((roper, holder_points)) = quad {
                let roper_body = f64::from(
                    roper.previous_body_yaw
                        + horizon_client::mobs::living::wrap(
                            roper.body_yaw - roper.previous_body_yaw,
                        ) * alpha as f32,
                )
                .to_radians();
                let holder_pos = lerp3(roper.previous, roper.position);
                let here = lerp3(m.previous, m.position);
                for (mine, theirs) in leash::quad_leash_offsets(m).iter().zip(holder_points) {
                    let start = here + rot_y(glam::DVec3::from_array(*mine), -body);
                    let end = holder_pos + rot_y(glam::DVec3::from_array(theirs), -roper_body);
                    ends.push((start, end, false));
                }
            } else {
                let offset = leash::leash_offset(m);
                let offset = rot_y(glam::DVec3::from_array(offset), -body);
                let start = lerp3(m.previous, m.position) + offset;
                let end = match holder {
                    Holder::Knot(p) => {
                        if !knots.contains(&p) {
                            knots.push(p);
                        }
                        glam::DVec3::new(
                            f64::from(p[0]) + 0.5,
                            f64::from(p[1]) + 0.375 + 0.2,
                            f64::from(p[2]) + 0.5,
                        )
                    }
                    Holder::Mob(id) => {
                        let Some(o) = self.mobs.iter().find(|o| o.id == id) else {
                            continue;
                        };
                        lerp3(o.previous, o.position)
                            + glam::DVec3::new(0.0, o.eye_height() * 0.7, 0.0)
                    }
                    Holder::Player => {
                        let Some(p) = self.player.as_ref() else {
                            continue;
                        };
                        self.player_rope_hold(p, alpha)
                    }
                };
                ends.push((start, end, true));
            }
            let light = |p: glam::DVec3| {
                self.world
                    .light(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
            };
            // Block and sky light averaged between the two ends.
            let (a, b) = (light(ends[0].0), light(ends[0].1));
            let packed = pack_light((((a >> 4) + (b >> 4)) / 2) << 4 | (((a & 15) + (b & 15)) / 2));
            for (start, end, slack) in ends {
                let d = end - start;
                let k = 1.0 / (d.x * d.x + d.z * d.z).sqrt().max(1e-6) * 0.05 / 2.0;
                let (dx_off, dz_off) = (d.z * k, d.x * k);
                let point = |step: i32, fudge: f64| {
                    let progress = f64::from(step) / 24.0;
                    // LeashFeatureRenderer.addVertexPair: a quad lead is straight.
                    let y = if !slack {
                        d.y * progress
                    } else if d.y > 0.0 {
                        d.y * progress * progress
                    } else {
                        d.y - d.y * (1.0 - progress) * (1.0 - progress)
                    };
                    let base = start + glam::DVec3::new(d.x * progress, y, d.z * progress);
                    (
                        base + glam::DVec3::new(-dx_off, fudge, dz_off),
                        base + glam::DVec3::new(dx_off, 0.05 - fudge, -dz_off),
                    )
                };
                for (fudge, backwards) in [(0.05, false), (0.0, true)] {
                    for step in 0..24 {
                        let shade = |s: i32| {
                            let dark = s % 2 == i32::from(backwards);
                            let f = if dark { 0.7 } else { 1.0 };
                            abgr(0.5 * f, 0.4 * f, 0.3 * f)
                        };
                        let (a0, b0) = point(step, fudge);
                        let (a1, b1) = point(step + 1, fudge);
                        let color = shade(step);
                        // Both faces: the leash is not culled.
                        for quad in [[a0, b0, b1, a1], [a1, b1, b0, a0]] {
                            for (v, uv) in quad.iter().zip(uv) {
                                out.push(Vertex {
                                    normal: [0; 4],
                                    pos: (*v - camera).as_vec3().to_array(),
                                    uv,
                                    layer: 0,
                                    color,
                                    light: packed,

                                    glint_uv: [0.0, 0.0],
                                });
                            }
                        }
                    }
                }
            }
        }
        // The knots: a 6 by 8 by 6 pixel block on the fence post.
        static KNOT: std::sync::OnceLock<Option<horizon_client::mobs::Part>> =
            std::sync::OnceLock::new();
        let model = KNOT.get_or_init(|| {
            horizon_client::mobs::load_model(&self.options.resources, "minecraft:leash_knot#main")
        });
        let Some(model) = model else { return };
        let Some(r) = atlas.sprites.get("entity/lead_knot/lead_knot").copied() else {
            return;
        };
        for p in knots {
            let state = leash::knot_state(p);
            let at =
                glam::DVec3::new(state.position.x, state.position.y, state.position.z) - camera;
            let base = glam::Mat4::from_translation(at.as_vec3())
                * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0));
            let poses = horizon_client::mobs::anim::Poses::new(model);
            let light = pack_light(self.world.light(p[0], p[1], p[2]));
            horizon_client::mobs::render::draw(
                model,
                base,
                &poses,
                [r.u0, r.v0, r.u1, r.v1],
                0xFFFF_FFFF,
                light,
                0,
                out,
            );
        }
    }
    /// `Player.getRopeHoldPosition` (and `LocalPlayer`'s in first person).
    fn player_rope_hold(
        &self,
        p: &horizon_client::motion::PlayerMotion,
        alpha: f64,
    ) -> glam::DVec3 {
        let pos = glam::DVec3::new(p.position.x, p.position.y, p.position.z);
        let _ = alpha;
        if self.camera_type.first_person() {
            let (yaw, pitch) = (
                f64::from(self.yaw.to_radians()),
                f64::from(self.pitch.to_radians()),
            );
            let v = glam::DVec3::new(-0.39, -0.6, 0.3);
            // Vec3.xRot(-pitch) then yRot(-yaw).
            let (s, c) = ((-pitch).sin(), (-pitch).cos());
            let v = glam::DVec3::new(v.x, v.y * c + v.z * s, v.z * c - v.y * s);
            let eye = pos + glam::DVec3::new(0.0, f64::from(p.dimensions.eye_height), 0.0);
            return rot_y(v, -yaw) + eye;
        }
        let yaw = f64::from(self.yaw.to_radians());
        let crouching = self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft);
        let v = glam::DVec3::new(
            -0.22,
            f64::from(p.dimensions.height) - 1.0,
            if crouching { -0.2 } else { 0.07 },
        );
        pos + rot_y(v, -yaw)
    }
}
