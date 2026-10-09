//! Shields and other `blocks_attacks` items: raising them (`ShieldItem.use`,
//! from either hand), `LivingEntity.getItemBlockingWith` and
//! `applyItemBlocking` inputs, and what follows a block: item damage,
//! the block sound, attacker knockback (`blockedByItem`) and axes disabling
//! the shield (`Player.blockUsingItem`, `BlocksAttacks.disable`).
use super::Game;
use horizon_client::{
    health::{BlockOutcome, Blocking},
    items::ItemStack,
};
use horizon_core::random::RandomSource;
use horizon_world::blocking_rules::{BlockingRules, DamageReduction, ItemDamageFunction};

/// `Inventory.SLOT_OFFHAND`: the `using` slot of an off-hand item.
pub(super) const OFFHAND_SLOT: usize = 40;

impl Game<'_> {
    /// The stack in an inventory slot, the off hand at `OFFHAND_SLOT`.
    pub(super) fn stack_in(&self, slot: usize) -> Option<&ItemStack> {
        if slot == OFFHAND_SLOT {
            self.offhand.as_ref()
        } else {
            self.inventory.get(slot)?.as_ref()
        }
    }
    fn stack_in_mut(&mut self, slot: usize) -> &mut Option<ItemStack> {
        if slot == OFFHAND_SLOT {
            &mut self.offhand
        } else {
            &mut self.inventory[slot]
        }
    }
    fn blocks_attacks(&self, stack: &ItemStack) -> Option<&horizon_client::items::BlocksAttacks> {
        self.res.items.get(&stack.item)?.blocks_attacks.as_ref()
    }
    /// `ShieldItem.use` (ItemUtils.startUsingInstantly) for the hand at `slot`;
    /// false when that hand holds no blocking item or it is on cooldown.
    pub(super) fn raise_shield(&mut self, slot: usize) -> bool {
        let Some(stack) = self.stack_in(slot) else {
            return false;
        };
        if self.blocks_attacks(stack).is_none() {
            return false;
        }
        if self.shield_cooldown == 0 && self.using.is_none() {
            self.using = Some((slot, 0));
        }
        true
    }
    /// Main-hand use fell through: try the off hand (Minecraft.startUseItem).
    pub(super) fn use_offhand(&mut self) -> bool {
        self.raise_shield(OFFHAND_SLOT)
    }
    /// `LivingEntity.getItemBlockingWith` and the inputs of `applyItemBlocking`
    /// for an attack from `from` (`DamageSource.getSourcePosition`).
    pub(super) fn player_blocking(
        &self,
        from: Option<[f64; 3]>,
        damage_type: &str,
        pierce: bool,
    ) -> Option<Blocking> {
        let (slot, ticks) = self.using?;
        let component = self.blocks_attacks(self.stack_in(slot)?)?;
        // BlocksAttacks.blockDelayTicks: Math.round(seconds * 20).
        if (ticks as i32) < (component.delay_seconds * 20.0).round() as i32 {
            return None;
        }
        let tags = &self.res.stations.enchantments.damage_tags;
        let matches = |spec: &str| match spec.strip_prefix('#') {
            Some(tag) => tags.get(tag).is_some_and(|t| t.contains(damage_type)),
            None => spec == damage_type,
        };
        let p = self.player.as_ref()?;
        let angle = match from {
            Some(src) => {
                // calculateViewVector(0, yHeadRot) against the flattened direction.
                let yaw = (-self.yaw).to_radians();
                let view = [yaw.sin() as f64, yaw.cos() as f64];
                let (dx, dz) = (src[0] - p.position.x, src[2] - p.position.z);
                let len = (dx * dx + dz * dz).sqrt();
                let to = if len < 1.0e-4 {
                    [0.0, 0.0]
                } else {
                    [dx / len, dz / len]
                };
                (to[0] * view[0] + to[1] * view[1]).acos()
            }
            None => f64::from(std::f32::consts::PI),
        };
        let (threshold, base, factor) = component.item_damage;
        Some(Blocking {
            rules: BlockingRules {
                delay_seconds: component.delay_seconds,
                disable_scale: component.disable_scale,
                reductions: component
                    .reductions
                    .iter()
                    .map(|r| DamageReduction {
                        horizontal_angle: r.horizontal_angle,
                        types: r.types.as_ref().map(|t| t.iter().any(|s| matches(s))),
                        base: r.base,
                        factor: r.factor,
                    })
                    .collect(),
                item_damage: ItemDamageFunction {
                    threshold,
                    base,
                    factor,
                },
            },
            bypassed: component.bypassed_by.as_deref().is_some_and(matches),
            pierce,
            angle,
        })
    }
    /// After a hurt with the shield raised: `hurtBlockingItem`, `onBlocked`
    /// and, for melee from a mob, `blockUsingItem`.
    pub(super) fn after_block(
        &mut self,
        outcome: BlockOutcome,
        damage: f32,
        attacker: Option<u32>,
        projectile: bool,
    ) {
        let Some((slot, _)) = self.using else { return };
        let Some(stack) = self.stack_in(slot).cloned() else {
            return;
        };
        let Some(component) = self.blocks_attacks(&stack).cloned() else {
            return;
        };
        let at = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        if outcome.sound
            && let Some(sound) = &component.block_sound
        {
            let pitch = 0.8 + self.world.random.next_float() * 0.4;
            self.audio.play(sound, at, 1.0, pitch);
        }
        if outcome.blocked <= 0.0 {
            return;
        }
        // ItemDamageFunction.apply, then hurtAndBreak.
        let (threshold, base, factor) = component.item_damage;
        let item_damage = if outcome.blocked < threshold {
            0
        } else {
            (base + factor * outcome.blocked).floor() as i32
        };
        if item_damage > 0 && !self.creative {
            let loss = self.durability_loss(&stack, item_damage as u32);
            let max = self.res.items.get(&stack.item).map_or(0, |i| i.max_damage);
            let cell = self.stack_in_mut(slot);
            if let Some(s) = cell.as_mut()
                && max > 0
            {
                s.damage += loss;
                if s.damage >= max {
                    *cell = None;
                    self.using = None;
                    self.audio.play(
                        "minecraft:item.shield.break",
                        at,
                        0.8,
                        0.8 + self.world.random.next_float() * 0.4,
                    );
                    return;
                }
            }
        }
        if projectile {
            return;
        }
        let Some(id) = attacker else { return };
        let fully = outcome.blocked >= damage;
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let (mx, mz) = (m.position.x, m.position.z);
        let disable = m
            .extra
            .main_hand
            .as_deref()
            .and_then(|item| self.res.items.get(item))
            .map_or(0.0, |i| i.disable_blocking_seconds);
        // Ravager.blockedByItem: stunned half the time (the defender pushed
        // apart), else the defender flung away; no roar in progress.
        if m.species() == horizon_client::mobs::Species::Ravager {
            let mut events = Vec::new();
            let roaring = m.extra.anim_ticks != 0;
            let stunned = self
                .mobs
                .iter_mut()
                .find(|m| m.id == id)
                .is_some_and(|m| horizon_client::mobs::ravager::blocked(m, &mut events));
            if !roaring && let Some(p) = self.player.as_mut() {
                let (xd, zd) = (p.position.x - mx, p.position.z - mz);
                let push = if stunned {
                    // Entity.push(entity): apart by 0.05 per block of overlap.
                    let d = (xd * xd + zd * zd).sqrt().max(0.01);
                    let k = (1.0 / d).min(1.0) * 0.05;
                    horizon_core::geometry::Vec3::new(xd / d * k, 0.0, zd / d * k)
                } else {
                    let dd = (xd * xd + zd * zd).max(0.001);
                    horizon_core::geometry::Vec3::new(xd / dd * 4.0, 0.2, zd / dd * 4.0)
                };
                p.velocity = p.velocity.add_vector(push);
            }
            let _ = self.handle_mob_events(events);
            return;
        }
        // LivingEntity.blockedByItem: defender.knockback(0.5, defender - attacker),
        // which moves the defender along -(xd, zd).
        if !fully && let Some(p) = self.player.as_mut() {
            let (xd, zd) = (p.position.x - mx, p.position.z - mz);
            let len = (xd * xd + zd * zd).sqrt();
            if len >= 1.0e-5 {
                let (dx, dz) = (xd / len * 0.5, zd / len * 0.5);
                let v = p.velocity;
                let y = if p.ground.on_ground {
                    (v.y / 2.0 + 0.5).min(0.4)
                } else {
                    v.y
                };
                p.velocity = horizon_core::geometry::Vec3::new(v.x / 2.0 - dx, y, v.z / 2.0 - dz);
            }
        }
        // BlocksAttacks.disable: item cooldown and stopUsingItem.
        let ticks = disable * component.disable_scale;
        let ticks = if ticks > 0.0 {
            (ticks * 20.0).round() as u32
        } else {
            0
        };
        if ticks > 0 {
            self.shield_cooldown = ticks;
            self.using = None;
            if let Some(sound) = &component.disabled_sound {
                let pitch = 0.8 + self.world.random.next_float() * 0.4;
                self.audio.play(sound, at, 0.8, pitch);
            }
        }
    }
}

/// The shield in a slot: `ShieldSpecialRenderer`'s plate and handle boxes
/// under item/shield's `gui` display transform, flat-lit and depth-sorted.
pub(super) fn draw_shield_icon(
    batch: &mut horizon_client::hud::HudBatch,
    atlas: &horizon_client::hud::HudAtlas,
    x: f32,
    y: f32,
) {
    let Some(r) = atlas
        .sprites
        .get("entity/shield/shield_base_nopattern")
        .copied()
    else {
        return;
    };
    // display.gui: rotation [15, -25, -5], translation [2, 3, 0], scale 0.65.
    let (sx, cx) = 15f32.to_radians().sin_cos();
    let (sy, cy) = (-25f32).to_radians().sin_cos();
    let (sz, cz) = (-5f32).to_radians().sin_cos();
    // Quaternionf.rotationXYZ: Rx * Ry * Rz.
    let rotate = |[x, y, z]: [f32; 3]| {
        let (x, y) = (x * cz - y * sz, x * sz + y * cz);
        let (x, z) = (x * cy + z * sy, -x * sy + z * cy);
        let (y, z) = (y * cx - z * sx, y * sx + z * cx);
        [x, y, z]
    };
    // Model pixels -> slot pixels: S(1,-1,-1), T(-0.5), S(0.65), R, T, and
    // the GUI's (16, -16, 16) around the slot centre.
    let place = |[px, py, pz]: [f32; 3]| {
        let q = [px / 16.0 - 0.5, -py / 16.0 - 0.5, -pz / 16.0 - 0.5].map(|v| v * 0.65);
        let [qx, qy, qz] = rotate(q);
        [
            x + 8.0 + (qx + 2.0 / 16.0) * 16.0,
            y + 8.0 - (qy + 3.0 / 16.0) * 16.0,
            qz,
        ]
    };
    let tex = |u: f32, v: f32| {
        [
            r.u0 + (r.u1 - r.u0) * u / 64.0,
            r.v0 + (r.v1 - r.v0) * v / 64.0,
        ]
    };
    // ShieldModel: plate texOffs(0, 0) box(-6, -11, -2, 12, 22, 1) and
    // handle texOffs(26, 0) box(-1, -3, -1, 2, 6, 6).
    let mut faces: Vec<(f32, [[f32; 2]; 4], [[f32; 2]; 4], u32)> = Vec::new();
    for (tu, tv, from, size) in [
        (0.0, 0.0, [-6.0, -11.0, -2.0], [12.0, 22.0, 1.0]),
        (26.0, 0.0, [-1.0, -3.0, -1.0], [2.0, 6.0, 6.0]),
    ] {
        let [w, h, d] = size;
        let (x0, y0, z0) = (from[0], from[1], from[2]);
        let (x1, y1, z1) = (x0 + w, y0 + h, z0 + d);
        let t0 = [x0, y0, z0];
        let t1 = [x1, y0, z0];
        let t2 = [x1, y1, z0];
        let t3 = [x0, y1, z0];
        let l0 = [x0, y0, z1];
        let l1 = [x1, y0, z1];
        let l2 = [x1, y1, z1];
        let l3 = [x0, y1, z1];
        let (u0, u1, u2) = (tu, tu + d, tu + d + w);
        let (u22, u3, u4) = (tu + d + w + w, tu + d + w + d, tu + d + w + d + w);
        let (v0, v1, v2) = (tv, tv + d, tv + d + h);
        // ModelPart.Cube's polygons: (vertices, u0, v0, u1, v1, normal).
        for (verts, a, b, c, e, n) in [
            ([l1, l0, t0, t1], u1, v0, u2, v1, [0.0, -1.0, 0.0]),
            ([t2, t3, l3, l2], u2, v1, u22, v0, [0.0, 1.0, 0.0]),
            ([t0, l0, l3, t3], u0, v1, u1, v2, [-1.0, 0.0, 0.0]),
            ([t1, t0, t3, t2], u1, v1, u2, v2, [0.0, 0.0, -1.0]),
            ([l1, t1, t2, l2], u2, v1, u3, v2, [1.0, 0.0, 0.0]),
            ([l0, l1, l2, l3], u3, v1, u4, v2, [0.0, 0.0, 1.0]),
        ] {
            let normal = rotate([n[0], -n[1], -n[2]]);
            if normal[2] <= 0.0 {
                continue;
            }
            let p = verts.map(place);
            let depth = p.iter().map(|q| q[2]).sum::<f32>() / 4.0;
            let uv = [tex(c, b), tex(a, b), tex(a, e), tex(c, e)];
            // Flat item lighting: faces toward the viewer are brightest.
            let shade = (0.6 + 0.4 * normal[2]).min(1.0);
            let s = (shade * 255.0) as u8;
            let color = horizon_client::hud::rgba(s, s, s, 255);
            faces.push((depth, p.map(|q| [q[0], q[1]]), uv, color));
        }
    }
    faces.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, p, uv, color) in faces {
        batch.quad(p, uv, color);
    }
}
