//! Crossbows (`CrossbowItem`): charging with the start/middle/end sounds
//! (Quick Charge), loading arrows into `charged_projectiles` (Multishot
//! loads three), and shooting critical arrows at power 3.15 with the fanned
//! Multishot angles and their shot pitches.
use super::{Game, view_vector};
use horizon_client::{
    items::ItemStack,
    mobs::Who,
    projectiles::{Projectile, ProjectileKind},
};
use horizon_core::{geometry::Vec3, random::RandomSource};

const CROSSBOW: &str = "minecraft:crossbow";

impl Game<'_> {
    fn charge_ticks(stack: &ItemStack) -> u32 {
        // crossbow_charge_time: 1.25 s, minus 0.25 s per Quick Charge level.
        let seconds = 1.25 - 0.25 * stack.enchantment_level("minecraft:quick_charge") as f32;
        (seconds.max(0.0) * 20.0).floor() as u32
    }
    fn charging_sounds(stack: &ItemStack) -> [&'static str; 3] {
        match stack.enchantment_level("minecraft:quick_charge") {
            0 => [
                "minecraft:item.crossbow.loading_start",
                "minecraft:item.crossbow.loading_middle",
                "minecraft:item.crossbow.loading_end",
            ],
            1 => [
                "minecraft:item.crossbow.quick_charge_1",
                "",
                "minecraft:item.crossbow.loading_end",
            ],
            2 => [
                "minecraft:item.crossbow.quick_charge_2",
                "",
                "minecraft:item.crossbow.loading_end",
            ],
            _ => [
                "minecraft:item.crossbow.quick_charge_3",
                "",
                "minecraft:item.crossbow.loading_end",
            ],
        }
    }
    fn arrow_slot(&self) -> Option<usize> {
        self.inventory
            .iter()
            .position(|s| s.as_ref().is_some_and(|s| &*s.item == "minecraft:arrow"))
    }
    /// `CrossbowItem.use`: shoot when charged, else start charging.
    pub(super) fn use_crossbow(&mut self, stack: &ItemStack) -> bool {
        if &*stack.item != CROSSBOW {
            return false;
        }
        // Use is not re-triggered while the crossbow is still held charging.
        if self.using.is_some() {
            return true;
        }
        if stack.data().is_some_and(|d| !d.charged.is_empty()) {
            self.shoot_crossbow();
        } else if (self.arrow_slot().is_some() || self.creative) && self.using.is_none() {
            self.using = Some((self.selected, 0));
        }
        true
    }
    /// `CrossbowItem.onUseTick` for the charge in progress.
    pub(super) fn tick_crossbow(&mut self, slot: usize, ticks: u32, stack: &ItemStack) {
        let ticks = ticks + 1;
        self.using = Some((slot, ticks));
        let charge = Self::charge_ticks(stack).max(1) as f32;
        let (before, now) = ((ticks - 1) as f32 / charge, ticks as f32 / charge);
        let sounds = Self::charging_sounds(stack);
        let at = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        if before < 0.2 && now >= 0.2 && !sounds[0].is_empty() {
            self.audio.play(sounds[0], at, 0.5, 1.0);
        }
        if before < 0.5 && now >= 0.5 && !sounds[1].is_empty() {
            self.audio.play(sounds[1], at, 0.5, 1.0);
        }
        let charged = stack.data().is_some_and(|d| !d.charged.is_empty());
        if now >= 1.0 && !charged {
            // tryLoadProjectiles / ProjectileWeaponItem.draw.
            let shots = if stack.enchantment_level("minecraft:multishot") > 0 {
                3
            } else {
                1
            };
            if !self.creative {
                let Some(i) = self.arrow_slot() else { return };
                if let Some(s) = self.inventory[i].as_mut() {
                    s.count -= 1;
                    if s.count == 0 {
                        self.inventory[i] = None;
                    }
                }
            }
            if let Some(bow) = self.inventory[slot].as_mut() {
                bow.data_mut().charged = vec![ItemStack::new("minecraft:arrow", 1); shots];
            }
            let pitch = 1.0 / (self.world.random.next_float() * 0.5 + 1.0) + 0.2;
            self.audio.play(sounds[2], at, 1.0, pitch);
        }
    }
    /// `CrossbowItem.performShooting`.
    fn shoot_crossbow(&mut self) {
        let Some(eye) = self.eye_position() else {
            return;
        };
        let Some(bow) = self.inventory[self.selected].as_mut() else {
            return;
        };
        let charged = std::mem::take(&mut bow.data_mut().charged);
        bow.tidy();
        let count = charged.len();
        // ProjectileWeaponItem.shoot: angles fanned 10 degrees apart.
        let spread = if count > 1 { 10.0 } else { 0.0 };
        let view = glam::DVec3::from_array(view_vector(self.yaw, self.pitch));
        let up = glam::DVec3::from_array(view_vector(self.yaw, self.pitch - 90.0));
        for (index, _) in charged.iter().enumerate() {
            let angle = if count > 1 {
                -spread + 2.0 * spread * index as f64 / (count - 1) as f64
            } else {
                0.0
            };
            let angle = if count == 3 {
                [0.0, -spread, spread][index]
            } else {
                angle
            };
            let dir =
                glam::DQuat::from_axis_angle(up.normalize_or_zero(), angle.to_radians()) * view;
            let kind = ProjectileKind::Arrow {
                base_damage: 2.0,
                critical: true,
                effect: None,
            };
            let mut arrow = Projectile::new(
                kind,
                Vec3::new(eye[0], eye[1] - 0.1, eye[2]),
                Some(Who::Player),
            );
            arrow.shoot(&mut self.world.random, dir.to_array(), 3.15, 1.0);
            self.creatures.projectiles.push(arrow);
            self.player_projectile_shot();
            let pitch = if index == 0 {
                1.0
            } else {
                let high = index & 1 == 1;
                1.0 / (self.world.random.next_float() * 0.5 + 1.8) + if high { 0.63 } else { 0.43 }
            };
            self.audio
                .play("minecraft:item.crossbow.shoot", Some(eye), 1.0, pitch);
            self.damage_held(1);
        }
    }
}
