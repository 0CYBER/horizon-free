//! Riding horses, donkeys and mules: feeding (`handleEating`), saddles,
//! armour and chests, mounting, the rider's control and jump bar
//! (`LocalPlayer.aiStep`'s riding jump), positioning, dismounting, and the
//! mount's health and jump bar on the HUD.
use super::Game;
use horizon_client::{
    hud::HudBatch,
    mobs::{MobEvent, Species, equine},
    sprint::Keys,
};
use horizon_core::{geometry::Vec3, random::RandomSource};

/// `passengerAttachments` minus the player's 0.6 vehicle attachment.
fn seat(s: Species) -> f64 {
    let attach = match s {
        Species::Horse => 1.44375,
        Species::Donkey => 1.1125,
        Species::Mule => 1.2125,
        _ => 1.31875,
    };
    attach - 0.6
}
#[derive(Default)]
pub(super) struct MountState {
    /// `jumpRidingTicks`: -10 cooling down after a jump, >= 0 charging.
    jump_ticks: i32,
    /// `jumpRidingScale` (0..1).
    pub(super) jump_scale: f32,
    was_jumping: bool,
}
impl Game<'_> {
    fn mount_index(&self) -> Option<usize> {
        let id = self.riding?;
        self.mobs.iter().position(|m| m.id == id)
    }
    /// `AbstractHorse.mobInteract`. Returns whether the click was used.
    pub(super) fn interact_mount(&mut self, index: usize, item: &str) -> bool {
        let m = &self.mobs[index];
        let species = m.species();
        if !equine::is_equine(species) || !m.alive() || Some(m.id) == self.riding {
            return false;
        }
        if self.feed_mount(index, item) {
            return true;
        }
        let m = &mut self.mobs[index];
        if m.is_baby() {
            return false;
        }
        let at = m.center();
        let tamed = m.extra.tame;
        let sound = if item == "minecraft:saddle" && tamed && !m.extra.saddled {
            m.extra.saddled = true;
            Some("minecraft:entity.horse.saddle")
        } else if item.ends_with("_horse_armor")
            && species == Species::Horse
            && tamed
            && m.extra.head.is_none()
        {
            m.extra.head = Some(item.into());
            Some("minecraft:entity.horse.armor")
        } else if item == "minecraft:chest"
            && matches!(species, Species::Donkey | Species::Mule)
            && tamed
            && !m.extra.chested
        {
            m.extra.chested = true;
            Some("minecraft:entity.donkey.chest")
        } else {
            None
        };
        if let Some(sound) = sound {
            let pitch =
                (self.world.random.next_float() - self.world.random.next_float()) * 0.2 + 1.0;
            self.audio.play(sound, Some(at), 0.5, pitch);
            self.consume_held();
            self.world
                .modified
                .insert((at[0].floor() as i32 >> 4, at[2].floor() as i32 >> 4));
            return true;
        }
        // doPlayerRide
        let m = &mut self.mobs[index];
        m.extra.flag = false;
        m.extra.counter = 0;
        self.yaw = m.yaw;
        self.riding = Some(m.id);
        self.mount.jump_ticks = 0;
        self.mount.jump_scale = 0.0;
        true
    }
    /// `AbstractHorse.handleEating`.
    fn feed_mount(&mut self, index: usize, item: &str) -> bool {
        let (heal, age_up, temper, love) = match item {
            "minecraft:wheat" => (2.0, 20, 3, false),
            "minecraft:sugar" => (1.0, 30, 3, false),
            "minecraft:hay_block" => (20.0, 180, 0, false),
            "minecraft:apple" => (3.0, 60, 3, false),
            "minecraft:red_mushroom" => (3.0, 0, 3, false),
            "minecraft:carrot" => (3.0, 60, 3, false),
            "minecraft:golden_carrot" => (4.0, 60, 5, true),
            "minecraft:golden_apple" | "minecraft:enchanted_golden_apple" => (10.0, 240, 10, true),
            _ => return false,
        };
        let m = &mut self.mobs[index];
        if matches!(m.species(), Species::SkeletonHorse | Species::ZombieHorse) {
            return false;
        }
        let mut used = false;
        if love && m.extra.tame && m.extra.age == 0 && m.extra.in_love <= 0 {
            m.extra.in_love = 600;
            m.extra.love_player = true;
            used = true;
        }
        if m.health < m.attrs.max_health && heal > 0.0 {
            m.health = (m.health + heal).min(m.attrs.max_health);
            used = true;
        }
        if m.is_baby() && age_up > 0 && !m.extra.age_locked {
            m.extra.age = (m.extra.age + age_up * 20).min(0);
            used = true;
        }
        if temper > 0 && (used || !m.extra.tame) && m.extra.variant3 < 100 {
            m.extra.variant3 = (m.extra.variant3 + temper).min(100);
            used = true;
        }
        if !used {
            return false;
        }
        // eating(): the mouth opens and the horse eats.
        m.anim.v[2] = 1.0;
        let at = m.center();
        let sound = match m.species() {
            Species::Donkey => "minecraft:entity.donkey.eat",
            Species::Mule => "minecraft:entity.mule.eat",
            _ => "minecraft:entity.horse.eat",
        };
        let pitch = (self.world.random.next_float() - self.world.random.next_float()) * 0.2 + 1.0;
        self.audio.play(sound, Some(at), 1.0, pitch);
        self.consume_held();
        true
    }
    /// The rider's input for this tick; returns false when not on a mount.
    pub(super) fn tick_mount_riding(&mut self, keys: Keys) -> bool {
        let Some(i) = self.mount_index() else {
            return false;
        };
        if keys.shift || !self.mobs[i].alive() {
            self.dismount_mount();
            return true;
        }
        let strafe = f32::from(u8::from(keys.left)) - f32::from(u8::from(keys.right));
        let forward = f32::from(u8::from(keys.forward)) - f32::from(u8::from(keys.backward));
        // LocalPlayer.aiStep: the riding jump charges while space is held.
        let saddled = self.mobs[i].extra.saddled;
        let mut jump = 0.0;
        if saddled {
            let st = &mut self.mount;
            if st.jump_ticks < 0 {
                st.jump_ticks += 1;
                if st.jump_ticks == 0 {
                    st.jump_scale = 0.0;
                }
            }
            if st.was_jumping && !keys.jump {
                st.jump_ticks = -10;
                let amount = (st.jump_scale * 100.0).floor() as i32;
                // PlayerRideableJumping.getPlayerJumpPendingScale
                jump = if amount >= 90 {
                    1.0
                } else {
                    0.4 + 0.4 * amount as f32 / 90.0
                };
            } else if !st.was_jumping && keys.jump {
                st.jump_ticks = 0;
                st.jump_scale = 0.0;
            } else if st.was_jumping {
                st.jump_ticks += 1;
                st.jump_scale = if st.jump_ticks < 10 {
                    st.jump_ticks as f32 * 0.1
                } else {
                    0.8 + 2.0 / (st.jump_ticks - 9) as f32 * 0.1
                };
            }
            st.was_jumping = keys.jump;
        }
        let m = &mut self.mobs[i];
        if jump > 0.0 {
            // onPlayerJump: rear while jumping (horses).
            if matches!(
                m.species(),
                Species::Horse | Species::SkeletonHorse | Species::ZombieHorse
            ) {
                m.extra.counter = 20;
            }
            let sound = "minecraft:entity.horse.jump";
            let at = m.center();
            self.audio.play(sound, Some(at), 0.4, 1.0);
        }
        let m = &mut self.mobs[i];
        let pending = m.rider.map_or(0.0, |r| r.jump);
        m.rider = Some(equine::Rider {
            strafe,
            forward,
            yaw: self.yaw,
            pitch: self.pitch,
            jump: if jump > 0.0 { jump } else { pending },
        });
        true
    }
    /// `positionRider`: the player sits on the mount after it moves.
    pub(super) fn position_rider(&mut self) {
        let Some(i) = self.mount_index() else {
            return;
        };
        let m = &self.mobs[i];
        let at = Vec3::new(m.position.x, m.position.y + seat(m.species()), m.position.z);
        if let Some(p) = self.player.as_mut() {
            self.previous_position = p.position;
            p.position = at;
            p.velocity = Vec3::ZERO;
            p.fall_distance = 0.0;
        }
    }
    /// `removePassenger` + `getDismountLocationForPassenger`: beside the mount.
    pub(super) fn dismount_mount(&mut self) {
        let Some(id) = self.riding.take() else {
            return;
        };
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        m.rider = None;
        let yaw = m.yaw.to_radians();
        let side = Vec3::new(f64::from(yaw.cos()), 0.0, f64::from(yaw.sin()));
        let w = f64::from(m.width()) / 2.0 + 0.4;
        let base = m.position;
        let candidates = [
            Vec3::new(base.x - side.x * w, base.y, base.z - side.z * w),
            Vec3::new(base.x + side.x * w, base.y, base.z + side.z * w),
            Vec3::new(base.x, base.y + f64::from(m.height()), base.z),
        ];
        let spot = candidates
            .into_iter()
            .find(|c| {
                let f = |dy: i32| {
                    self.res
                        .facts
                        .get(self.world.block(
                            c.x.floor() as i32,
                            c.y.floor() as i32 + dy,
                            c.z.floor() as i32,
                        ))
                        .full_collision
                };
                !f(0) && !f(1)
            })
            .unwrap_or(candidates[2]);
        if let Some(p) = self.player.as_mut() {
            p.position = spot;
            p.velocity = Vec3::ZERO;
        }
    }
    pub(super) fn mount_event(&mut self, e: &MobEvent) {
        match *e {
            MobEvent::Tamed { mob } => {
                let owner = self.level.player_uuid.or(Some([0, 0, 0, 1]));
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                    m.extra.owner = owner;
                    m.persistent = true;
                    let at = [
                        m.position.x,
                        m.position.y + f64::from(m.height()) * 0.5 + 0.5,
                        m.position.z,
                    ];
                    let _ = self.handle_mob_events(vec![MobEvent::Particles {
                        kind: "heart",
                        at,
                        count: 7,
                    }]);
                }
            }
            MobEvent::Buck { mob } if self.riding == Some(mob) => self.dismount_mount(),
            _ => {}
        }
    }
    /// `Gui.renderVehicleHealth` and the jump bar (`JumpableVehicleBarRenderer`).
    pub(super) fn draw_mount_hud(&self, batch: &mut HudBatch, w: f32, h: f32) -> bool {
        let Some(i) = self.mount_index() else {
            return false;
        };
        let m = &self.mobs[i];
        let atlas = &self.res.atlas;
        // Hearts in rows of ten, right-aligned where the food bar sits.
        let max = (m.attrs.max_health + 0.5) as i32 / 2;
        let health = m.health.ceil() as i32;
        let right = w / 2.0 + 91.0;
        let mut y = h - 39.0;
        let mut drawn = 0;
        while drawn < max {
            let row = (max - drawn).min(10);
            for k in 0..row {
                let x = right - k as f32 * 8.0 - 9.0;
                batch.sprite(atlas, "gui/sprites/hud/heart/vehicle_container", x, y);
                let v = health - (drawn + k) * 2;
                if v >= 2 {
                    batch.sprite(atlas, "gui/sprites/hud/heart/vehicle_full", x, y);
                } else if v == 1 {
                    batch.sprite(atlas, "gui/sprites/hud/heart/vehicle_half", x, y);
                }
            }
            drawn += row;
            y -= 10.0;
        }
        // The jump bar replaces the experience bar on a saddled jumper.
        if m.extra.saddled {
            let x = w / 2.0 - 91.0;
            let y = h - 29.0;
            batch.sprite(atlas, "gui/sprites/hud/jump_bar_background", x, y);
            if self.mount.jump_ticks < 0 {
                batch.sprite(atlas, "gui/sprites/hud/jump_bar_cooldown", x, y);
            } else {
                let filled = (self.mount.jump_scale * 183.0).floor();
                if filled > 0.0 {
                    batch.sprite_part(
                        atlas,
                        "gui/sprites/hud/jump_bar_progress",
                        x,
                        y,
                        filled / 182.0,
                    );
                }
            }
        }
        true
    }
}
