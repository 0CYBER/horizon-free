//! `ThrownTrident` (arrow flight with 0.99 water inertia, the reversed bounce off a hit, sticking,
//! Loyalty's return to the owner) and the pure rules around it: the Breeze deflection and the segment test.
use horizon_client::arrows::{Pickup, back_off};
use horizon_client::{items::ItemStack, projectiles::ProjectileWorld};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::{EntityRandom, RandomSource},
};

pub struct ThrownTrident {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: horizon_mod_runtime::attachments::AttachmentLifetime,
    /// When it joined its list (`horizon_client::join_seq`); set by the list.
    pub join: horizon_client::join_seq::JoinSeq,
    /// The entity id a server tracks it by (0 until it is tracked).
    pub id: u32,
    pub stack: ItemStack,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub needs_sync: bool,
    pub yaw: f32,
    pub pitch: f32,
    pub in_ground: bool,
    /// `AbstractArrow.inGroundTime`: the ticks it has been stuck, from 0 on
    /// load; past 4, Loyalty calls the trident back.
    pub in_ground_time: i32,
    /// `AbstractArrow.life`, the saved despawn count: kept apart from
    /// `in_ground_time`, as loaded (negative too).
    pub life: i32,
    /// `AbstractArrow.shakeTime`: seven ticks after sticking, no pickup.
    pub shake: i32,
    pub dealt_damage: bool,
    pub loyalty: u32,
    pub no_physics: bool,
    pub returning: u32,
    pub pickup: Pickup,
    pub left_owner: bool,
    pub has_been_shot: bool,
    pub owner_uuid: Option<[i32; 4]>,
    /// `Entity.isNoGravity`: `AbstractArrow.applyGravity` then skips the pull.
    pub no_gravity: bool,
    /// `Entity.onGround`: moved by `setPos`, it keeps the one it was loaded with.
    pub on_ground: bool,
    /// `Entity.remainingFireTicks`; on clients, the synchronized on-fire flag.
    pub fire_ticks: i32,
    pub random: EntityRandom,
    /// `Entity.fallDistance`: kept as loaded; water resets it.
    pub fall_distance: f64,
    /// `AbstractArrow.soundEvent`: the trident's own until it first hits a block.
    pub sound_event: String,
    /// Water membership from the preceding `baseTick` (not updated while stuck).
    pub was_touching_water: bool,
    /// `Entity.random`, used for the extinguishing sound and a breeze's turn.
    /// `AbstractArrow.lastState`: the block it stuck in; not saved.
    pub last_state: Option<horizon_world::registry::BlockStateId>,
    /// `Projectile.lastDeflectedBy`: the breeze that already turned this trident.
    pub deflected_by: Option<u32>,
    /// `Entity.firstTick`: cleared by `baseTick`, which a stuck trident never reaches.
    pub first_tick: bool,
    /// A mod canceled its block hit and it stands inside that shape: it flies
    /// on until it has left it.
    pub canceled_impact_inside: bool,
    /// `Entity.tickCount`: `ProjectileUtil.computeMargin` grows with it.
    pub age: u32,
    /// The entity id a server sent as its owner (the one Loyalty returns it to).
    pub owner: u32,
    /// The guest that threw it (`Projectile.getOwner`); `None` for the host.
    pub guest: Option<u32>,
}
impl ThrownTrident {
    /// A trident loaded from its NBT (`AbstractArrow.readAdditionalSaveData`).
    pub fn from_nbt(
        n: horizon_client::entity_nbt::TridentNbt,
        tag: &horizon_core::nbt::Tag,
        registry: &horizon_world::registry::BlockRegistry,
    ) -> Self {
        let mut t = Self::remote(0, n.position, n.yaw, n.pitch);
        t.load_nbt(n, tag, registry);
        t
    }
    /// `ThrownTrident.readAdditionalSaveData` on a new or resident entity;
    /// `/data` keeps its runtime identity, random and tick count.
    pub fn load_nbt(
        &mut self,
        n: horizon_client::entity_nbt::TridentNbt,
        tag: &horizon_core::nbt::Tag,
        registry: &horizon_world::registry::BlockRegistry,
    ) {
        // `inBlockState` (`BlockState.CODEC`) is `lastState`; absent or
        // unreadable reads as null.
        self.last_state = tag
            .get("inBlockState")
            .and_then(|s| horizon_client::entity_nbt::state_from_nbt(registry, s));
        self.position = n.position;
        self.previous = n.position;
        self.yaw = n.yaw;
        self.pitch = n.pitch;
        self.loyalty = n.item.enchantment_level("minecraft:loyalty");
        self.stack = n.item;
        self.velocity = n.motion;
        self.in_ground = n.in_ground;
        self.life = n.life;
        self.in_ground_time = 0;
        self.shake = n.shake;
        self.dealt_damage = n.dealt_damage;
        self.pickup = n.pickup;
        self.left_owner = n.left_owner;
        self.has_been_shot = n.has_been_shot;
        self.owner_uuid = n.owner;
        self.guest = None;
        self.no_gravity = n.no_gravity;
        self.on_ground = n.on_ground;
        self.fire_ticks = n.fire_ticks;
        self.fall_distance = n.fall_distance;
        self.sound_event = n.sound_event;
    }
    /// `AbstractArrow.addAdditionalSaveData` + `DealtDamage`.
    pub fn to_nbt(
        &self,
        uuid: [i32; 4],
        registry: &horizon_world::registry::BlockRegistry,
        raw: Option<&horizon_core::nbt::Tag>,
    ) -> horizon_core::nbt::Tag {
        let mut tag = horizon_client::entity_nbt::trident_to_nbt(
            &horizon_client::entity_nbt::TridentSave {
                position: self.position,
                motion: self.velocity,
                yaw: self.yaw,
                pitch: self.pitch,
                in_ground: self.in_ground,
                life: self.life,
                shake: self.shake,
                dealt_damage: self.dealt_damage,
                pickup: self.pickup,
                left_owner: self.left_owner,
                has_been_shot: self.has_been_shot,
                owner: self.owner_uuid,
                no_gravity: self.no_gravity,
                on_ground: self.on_ground,
                fire_ticks: self.fire_ticks,
                fall_distance: self.fall_distance,
                sound_event: &self.sound_event,
                item: &self.stack,
            },
            uuid,
            raw,
        );
        // `AbstractArrow.addAdditionalSaveData`: `storeNullable("inBlockState")`,
        // never a stale value from the loaded compound.
        if let horizon_core::nbt::Tag::Compound(map) = &mut tag {
            map.remove(&"inBlockState".into());
            if let Some(state) = self.last_state {
                map.insert(
                    "inBlockState".into(),
                    horizon_client::entity_nbt::state_to_nbt(registry, state),
                );
            }
        }
        tag
    }
    /// A trident a server sent (`handleAddEntity`): placed and moved by it.
    pub fn remote(id: u32, position: Vec3, yaw: f32, pitch: f32) -> Self {
        Self {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            stack: ItemStack::new("minecraft:trident", 1),
            position,
            previous: position,
            velocity: Vec3::ZERO,
            needs_sync: false,
            yaw,
            pitch,
            in_ground: false,
            in_ground_time: 0,
            life: 0,
            shake: 0,
            dealt_damage: false,
            loyalty: 0,
            no_physics: false,
            returning: 0,
            pickup: Pickup::Disallowed,
            left_owner: false,
            has_been_shot: false,
            owner_uuid: None,
            no_gravity: false,
            on_ground: false,
            fire_ticks: 0,
            fall_distance: 0.0,
            sound_event: horizon_client::entity_nbt::TRIDENT_HIT_GROUND.to_owned(),
            was_touching_water: false,
            age: 0,
            owner: 0,
            guest: None,
            random: horizon_client::projectiles::unique_random(position),
            deflected_by: None,
            first_tick: true,
            canceled_impact_inside: false,
            last_state: None,
        }
    }
    /// The check that opens `AbstractArrow.tick`, with `inside` its answer: a
    /// position inside the block's collision shape stops the trident.
    /// `setInGround(true)` starts the shake (`onSyncedDataUpdated`), except on
    /// the first tick or over a shake still running.
    pub fn settle_inside_block(&mut self, inside: bool) {
        if inside && !self.canceled_impact_inside {
            self.velocity = Vec3::ZERO;
            if !self.in_ground && !self.first_tick && self.shake <= 0 {
                self.shake = 7;
            }
            self.in_ground = true;
        }
        self.canceled_impact_inside &= inside;
    }
    /// `Entity.noPhysics`, with the synced flag a client reads
    /// (`AbstractArrow`'s entity-data flags, bit 1): the tracker sends them
    /// again on the next tick.
    pub fn set_no_physics(&mut self, no_physics: bool) {
        self.no_physics = no_physics;
        self.needs_sync = true;
    }
    /// `Entity.move(PISTON)`: the position only; `release_after_piston` follows
    /// (`AbstractArrow.move`).
    pub fn move_by_piston(&mut self, movement: Vec3) {
        self.position = self.position.add_vector(movement);
        self.needs_sync = true;
    }
    pub fn piston_release_bounds(&self) -> Option<Aabb> {
        if !self.in_ground || self.no_physics {
            return None;
        }
        Some(Aabb::new(self.position, self.position).inflate(Vec3::new(0.06, 0.06, 0.06)))
    }
    pub fn release_after_piston(&mut self, no_block_collision: bool) {
        if self.in_ground && !self.no_physics && no_block_collision {
            self.start_falling();
        }
    }
    /// A slime block's bounce: the velocity only; the piston's move after it
    /// releases a stuck trident (`AbstractArrow.move`).
    pub fn piston_slime_bounce(&mut self, movement: horizon_core::geometry::Direction) {
        self.velocity = horizon_world::piston_push::bounce_velocity(movement, self.velocity);
        self.needs_sync = true;
    }
    /// `AbstractArrow.onHitBlock`: plays `getHitGroundSoundEvent()`, then
    /// `setSoundEvent(SoundEvents.ARROW_HIT)` for every later hit and the save.
    pub fn hit_ground_sound(&mut self) -> String {
        std::mem::replace(
            &mut self.sound_event,
            "minecraft:entity.arrow.hit".to_owned(),
        )
    }
    /// `AbstractArrow.tick` in flight, before it moves: the rotation turns a
    /// fifth of the way (`Projectile.lerpRotation`) toward the movement the
    /// tick starts with, which faces back while it flies without physics.
    pub fn turn_towards_movement(&mut self) {
        use horizon_core::geometry::trig::atan2_degrees;
        let m = self.velocity;
        let y_rot = if self.no_physics {
            atan2_degrees(-m.x, -m.z)
        } else {
            atan2_degrees(m.x, m.z)
        };
        let x_rot = atan2_degrees(m.y, m.horizontal_distance());
        self.pitch = horizon_client::arrows::lerp_rotation(self.pitch, x_rot);
        self.yaw = horizon_client::arrows::lerp_rotation(self.yaw, y_rot);
    }
    /// `AbstractArrow.shouldFall`: stuck, and nothing solid within 0.06 of it.
    pub fn should_fall(&self, scene: &horizon_client::arrows::ArrowScene<'_, '_>) -> bool {
        let at = self.position;
        self.in_ground
            && scene.no_block_collision(Aabb::new(at, at).inflate(Vec3::new(0.06, 0.06, 0.06)))
    }
    /// `ThrownTrident.tickDespawn`: a Loyalty trident that can be picked up never
    /// counts; any other adds one to `life` (`AbstractArrow.tickDespawn`) and
    /// is discarded at 1200. True when it is discarded.
    pub fn tick_despawn(&mut self) -> bool {
        if self.loyalty > 0 && self.pickup == Pickup::Allowed {
            return false;
        }
        self.life += 1;
        self.life >= 1200
    }
    /// `AbstractArrow.startFalling`: unstick, scaling the velocity by three
    /// draws of `nextFloat() * 0.2F` in x, y, z order.
    pub fn start_falling(&mut self) {
        self.in_ground = false;
        let v = self.velocity;
        let x = f64::from(self.random.next_float() * 0.2);
        let y = f64::from(self.random.next_float() * 0.2);
        let z = f64::from(self.random.next_float() * 0.2);
        self.velocity = Vec3::new(v.x * x, v.y * y, v.z * z);
        // `this.life = 0`; and a LAN client hears of it.
        self.life = 0;
        self.in_ground_time = 0;
        self.needs_sync = true;
    }
    /// `ThrownTrident.tick`'s Loyalty return: pulled up and towards the
    /// owner's eye.
    pub fn loyalty_pull(&mut self, owner_eye: Vec3) {
        let v = Vec3::new(
            owner_eye.x - self.position.x,
            owner_eye.y - self.position.y,
            owner_eye.z - self.position.z,
        );
        self.position.y += v.y * 0.015 * f64::from(self.loyalty);
        let len = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt().max(1e-9);
        let accel = 0.05 * f64::from(self.loyalty);
        self.velocity = Vec3::new(
            self.velocity.x * 0.95 + v.x / len * accel,
            self.velocity.y * 0.95 + v.y / len * accel,
            self.velocity.z * 0.95 + v.z / len * accel,
        );
    }
    /// `ThrownTrident.tick` on a server's client: it flies on by itself
    /// between the server's updates (`AbstractArrow.tick`: gravity and drag,
    /// a block holds it), and Loyalty pulls it back to its owner; only the
    /// server decides what it hits.
    pub fn remote_tick(&mut self, w: &impl ProjectileWorld, owner_eye: Option<Vec3>) {
        self.previous = self.position;
        if self.in_ground_time > 4 {
            self.dealt_damage = true;
        }
        if self.loyalty > 0
            && (self.dealt_damage || self.no_physics)
            && let Some(eye) = owner_eye
        {
            self.no_physics = true;
            self.in_ground = false;
            self.loyalty_pull(eye);
        }
        // AbstractArrow.tick opens here, before its `--shakeTime`.
        self.settle_inside_block(!self.no_physics && w.inside_collision(self.position));
        self.shake -= i32::from(self.shake > 0);
        // AbstractArrow.tick releases a changed block only on the server;
        // clients release from the non-SELF move after a piston push.
        if self.in_ground && !self.no_physics {
            self.in_ground_time += 1;
            return;
        }
        self.in_ground_time = 0;
        // A flight reaches `Entity.baseTick`, even one that hits a block below.
        self.first_tick = false;
        // The same turn as the host's tick: a fifth of the way, `Mth.atan2`.
        self.turn_towards_movement();
        let m = self.velocity;
        let to = self.position.add_vector(m);
        if !self.no_physics
            && let Some((at, _, _)) = w.clip(self.position, to)
        {
            // AbstractArrow.onHitBlock's back-off, as the server's tick does it.
            self.position = back_off(at, m);
            self.velocity = Vec3::ZERO;
            self.in_ground = true;
            self.shake = 7;
            return;
        }
        self.position = to;
        // A trident's water inertia (0.99) equals the air drag.
        self.velocity = self.velocity.scale(0.99);
        if !self.no_physics && !self.no_gravity {
            self.velocity.y -= 0.05;
        }
    }
}
pub const HALF: f64 = 0.25;
/// `Breeze.PROJECTILE_DEFLECTION`: `ProjectileDeflection.REVERSE` at power
/// `(1, 1, 1)`. Each axis is multiplied by `-0.5`, the yaw turns by
/// `170 + nextFloat * 20`, and `needsSync` sends that at once. A trident is
/// no `AbstractHurtingProjectile`, so `onDeflection` has no power to halve.
pub fn breeze_deflect(t: &mut ThrownTrident, jitter: f32) {
    t.velocity = Vec3::new(
        t.velocity.x * -0.5,
        t.velocity.y * -0.5,
        t.velocity.z * -0.5,
    );
    t.yaw += 170.0 + jitter * 20.0;
    t.needs_sync = true;
}
/// The entry distance of a segment into a box, if it crosses it.
pub fn segment_box(from: Vec3, d: Vec3, b: Aabb) -> Option<f64> {
    let (lo, hi) = (b.min_position(), b.max_position());
    let (o, d, lo, hi) = (
        [from.x, from.y, from.z],
        [d.x, d.y, d.z],
        [lo.x, lo.y, lo.z],
        [hi.x, hi.y, hi.z],
    );
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for a in 0..3 {
        if d[a].abs() < 1e-12 {
            if o[a] < lo[a] || o[a] > hi[a] {
                return None;
            }
            continue;
        }
        let (mut ta, mut tb) = ((lo[a] - o[a]) / d[a], (hi[a] - o[a]) / d[a]);
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
        }
        t0 = t0.max(ta);
        t1 = t1.min(tb);
    }
    (t0 <= t1).then_some(t0)
}
horizon_client::impl_joined!(ThrownTrident);
horizon_mod_runtime::impl_attachment_lifetime!(ThrownTrident);
impl horizon_client::entity_rules::EntityRules for ThrownTrident {}

#[cfg(test)]
mod tests {
    use super::{ThrownTrident, breeze_deflect};
    use horizon_client::{mobs::Who, projectiles::ProjectileWorld};
    use horizon_core::geometry::{Aabb, Vec3};

    struct Air;
    impl ProjectileWorld for Air {
        fn clip(&self, _: Vec3, _: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
            None
        }
        fn in_water(&self, _: Aabb) -> bool {
            false
        }
        fn targets(&self) -> &[(Who, Aabb)] {
            &[]
        }
    }

    /// A trident summoned at rest still reads pitch 0 after its first tick
    /// and -17.999998 after the second, when it starts falling; one summoned
    /// with Motion [0, 0.5, 0] reads 17.999998 after its first (#13168).
    #[test]
    fn a_flying_trident_turns_a_fifth_of_the_way_each_tick() {
        let mut t = ThrownTrident::remote(1, Vec3::ZERO, 0.0, 0.0);
        t.turn_towards_movement();
        assert_eq!((t.yaw, t.pitch), (0.0, 0.0));
        t.velocity = Vec3::new(0.0, -0.05, 0.0);
        t.turn_towards_movement();
        assert_eq!((t.yaw, t.pitch), (0.0, -17.999998));
        let mut up = ThrownTrident::remote(2, Vec3::ZERO, 0.0, 0.0);
        up.velocity = Vec3::new(0.0, 0.5, 0.0);
        up.turn_towards_movement();
        assert_eq!(up.pitch, 17.999998);
    }

    /// `AbstractArrow.tickDespawn` through `ThrownTrident.tickDespawn`: the tick
    /// that brings `life` to 1200 discards a stuck trident, one loaded past
    /// 1200 goes on its first count, and a Loyalty trident that can be picked
    /// up never counts.
    #[test]
    fn a_stuck_trident_is_discarded_when_life_reaches_1200() {
        use horizon_client::arrows::Pickup;
        let mut t = ThrownTrident::remote(1, Vec3::ZERO, 0.0, 0.0);
        t.life = 1198;
        assert!(!t.tick_despawn());
        assert_eq!(t.life, 1199);
        assert!(t.tick_despawn());
        assert_eq!(t.life, 1200);
        let mut loaded = ThrownTrident::remote(2, Vec3::ZERO, 0.0, 0.0);
        loaded.life = 5000;
        assert!(loaded.tick_despawn());
        // Loyalty only exempts a trident that can be picked up.
        let mut loyal = ThrownTrident::remote(3, Vec3::ZERO, 0.0, 0.0);
        (loyal.loyalty, loyal.pickup, loyal.life) = (3, Pickup::Allowed, 1199);
        assert!(!loyal.tick_despawn());
        assert_eq!(loyal.life, 1199);
        loyal.pickup = Pickup::CreativeOnly;
        assert!(loyal.tick_despawn());
        loyal.pickup = Pickup::Allowed;
        loyal.loyalty = 0;
        assert!(loyal.tick_despawn());
    }

    /// `AbstractArrow.startFalling` sets `life = 0`: a block removed under a
    /// trident that was about to despawn starts the count over.
    #[test]
    fn a_trident_that_starts_falling_restarts_its_life() {
        let mut t = ThrownTrident::remote(1, Vec3::ZERO, 0.0, 0.0);
        (t.in_ground, t.life) = (true, 1199);
        t.start_falling();
        assert_eq!((t.in_ground, t.life), (false, 0));
    }

    /// A block registry with air and stone, for save round trips.
    fn stone_registry() -> horizon_world::registry::BlockRegistry {
        use horizon_core::nbt::Tag;
        use horizon_world::registry::BlockRegistry;
        BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
            (
                "states",
                Tag::List(vec![
                    Tag::compound([("id", Tag::String("minecraft:air".into()))]),
                    Tag::compound([("id", Tag::String("minecraft:stone".into()))]),
                ]),
            ),
            (
                "defaults",
                Tag::compound([
                    ("minecraft:air", Tag::Int(0)),
                    ("minecraft:stone", Tag::Int(1)),
                ]),
            ),
        ]))
        .unwrap()
    }

    /// A trident hits its first block with its own sound and every later one
    /// with `entity.arrow.hit`, which is what it saves and keeps across a
    /// reload (#13172).
    #[test]
    fn a_trident_hits_with_its_own_sound_once() {
        use horizon_core::nbt::Tag;
        let registry = stone_registry();
        let saved =
            |t: &ThrownTrident| t.to_nbt([0; 4], &registry, None).get("SoundEvent").cloned();
        let sound = |id: &str| Some(Tag::String(id.into()));
        let mut t = ThrownTrident::remote(1, Vec3::ZERO, 0.0, 0.0);
        assert_eq!(saved(&t), sound("minecraft:item.trident.hit_ground"));
        assert_eq!(t.hit_ground_sound(), "minecraft:item.trident.hit_ground");
        assert_eq!(saved(&t), sound("minecraft:entity.arrow.hit"));
        let tag = t.to_nbt([0; 4], &registry, None);
        let n = horizon_client::entity_nbt::trident_from_nbt(&tag).unwrap();
        let mut back = ThrownTrident::from_nbt(n, &tag, &registry);
        assert_eq!(back.hit_ground_sound(), "minecraft:entity.arrow.hit");
        // readAdditionalSaveData: no SoundEvent, or no identifier, is the default.
        let pos = ("Pos", Tag::List(vec![Tag::Double(0); 3]));
        for tag in [
            Tag::compound([pos.clone()]),
            Tag::compound([pos, ("SoundEvent", Tag::String("Not An Id".into()))]),
        ] {
            let n = horizon_client::entity_nbt::trident_from_nbt(&tag).unwrap();
            assert_eq!(n.sound_event, "minecraft:item.trident.hit_ground");
        }
    }

    /// `AbstractArrow.readAdditionalSaveData` keeps a negative `life` as it
    /// was saved (it only counts up while the trident is in a block) and reads
    /// `shake` as `& 255`, so `-1b` is 255 ticks and saves as `-1b` again.
    #[test]
    fn a_trident_keeps_a_negative_life_and_shake_across_a_reload() {
        use horizon_core::nbt::Tag;
        let registry = stone_registry();
        let tag = Tag::compound([
            ("Pos", Tag::List(vec![Tag::Double(0); 3])),
            ("life", Tag::Short(-1)),
            ("shake", Tag::Byte(-1)),
        ]);
        let loaded = horizon_client::entity_nbt::trident_from_nbt(&tag).unwrap();
        assert_eq!((loaded.life, loaded.shake), (-1, 255));
        // `Entity.load` reads `Fire` with `getShortOr` as well: 40000 is -25536.
        let burning = Tag::compound([
            ("Pos", Tag::List(vec![Tag::Double(0); 3])),
            ("Fire", Tag::Int(40000)),
        ]);
        let fire = horizon_client::entity_nbt::trident_from_nbt(&burning)
            .unwrap()
            .fire_ticks;
        assert_eq!(fire, -25536);
        let back = ThrownTrident::from_nbt(loaded, &tag, &registry);
        // `inGroundTime` is not saved: Loyalty's count starts at 0 whatever
        // `life` was, so a negative `life` does not hold the trident back.
        assert_eq!((back.life, back.in_ground_time), (-1, 0));
        let saved = back.to_nbt([1; 4], &registry, None);
        assert_eq!(saved.get("life"), Some(&Tag::Short(-1)));
        assert_eq!(saved.get("shake"), Some(&Tag::Byte(-1)));
    }

    /// `AbstractArrow` saves `lastState` as `inBlockState` and reads it back.
    #[test]
    fn a_trident_round_trips_its_in_block_state() {
        use horizon_core::nbt::Tag;
        let registry = stone_registry();
        let mut t = ThrownTrident::remote(1, Vec3::ZERO, 0.0, 0.0);
        assert!(
            t.to_nbt([1; 4], &registry, None)
                .get("inBlockState")
                .is_none()
        );
        t.last_state = registry.default_state("minecraft:stone");
        let tag = t.to_nbt([1; 4], &registry, None);
        assert_eq!(
            tag.get("inBlockState"),
            Some(&Tag::String("minecraft:stone".into()))
        );
        let loaded = horizon_client::entity_nbt::trident_from_nbt(&tag).unwrap();
        let back = ThrownTrident::from_nbt(loaded, &tag, &registry);
        assert_eq!(back.last_state, t.last_state);
    }

    /// `Entity.saveWithoutId` writes the `fall_distance` `Entity.load` read:
    /// a trident keeps the one it was loaded with.
    #[test]
    fn a_trident_keeps_its_fall_distance_across_a_reload() {
        use horizon_core::nbt::Tag;
        let registry = stone_registry();
        let fall = |tag: &Tag| match tag.get("fall_distance") {
            Some(Tag::Double(bits)) => Some(f64::from_bits(*bits)),
            _ => None,
        };
        let fresh = ThrownTrident::remote(1, Vec3::ZERO, 0.0, 0.0);
        assert_eq!(fall(&fresh.to_nbt([1; 4], &registry, None)), Some(0.0));
        let tag = Tag::compound([
            ("Pos", Tag::List(vec![Tag::Double(0); 3])),
            ("fall_distance", Tag::Double(5.0_f64.to_bits())),
        ]);
        let loaded = horizon_client::entity_nbt::trident_from_nbt(&tag).unwrap();
        let back = ThrownTrident::from_nbt(loaded, &tag, &registry);
        assert_eq!(fall(&back.to_nbt([1; 4], &registry, None)), Some(5.0));
    }

    /// `AbstractArrow.startFalling` on `new Random(42)`: its first three
    /// `nextFloat() * 0.2F` draws (computed from Java's LCG) scale x, y, z.
    #[test]
    fn a_falling_trident_scales_its_velocity_by_three_draws() {
        let mut t = ThrownTrident::remote(42, Vec3::ZERO, 0.0, 0.0);
        // The network id does not seed the projectile's independent entity random.
        t.random =
            horizon_core::random::EntityRandom::own(horizon_core::random::LegacyRandom::new(42));
        t.in_ground = true;
        t.velocity = Vec3::new(1.0, 1.0, 1.0);
        t.start_falling();
        assert!(!t.in_ground);
        assert_eq!(
            t.velocity,
            Vec3::new(0.14551274478435516, 0.0109330415725708, 0.13664469122886658)
        );
    }

    /// `ProjectileDeflection.REVERSE` at power 1: half speed, the other way,
    /// a turn of 170 degrees plus a fifth of the random draw, and the turn
    /// sent at once instead of at the trident's next update interval.
    #[test]
    fn a_breeze_turns_a_trident_around_at_half_speed() {
        let mut t = ThrownTrident::remote(42, Vec3::ZERO, 0.0, 0.0);
        t.velocity = Vec3::new(4.0, -2.0, 1.0);
        t.yaw = 10.0;
        assert!(!t.needs_sync);
        breeze_deflect(&mut t, 0.25);
        assert_eq!(t.velocity, Vec3::new(-2.0, 1.0, -0.5));
        assert_eq!(t.yaw, 185.0);
        assert!(t.needs_sync, "the turn goes out at once");
    }

    /// `AbstractArrow.tick` runs on a guest too: before the move the angles
    /// turn a fifth of the way (`lerpRotation`) toward the motion, which faces
    /// back without physics (a Loyalty trident coming home). Along (1, 0, 1)
    /// that is yRot 45 and xRot 0, so from (0, 10) a fifth: (9, 8).
    #[test]
    fn a_remote_trident_turns_a_fifth_of_the_way_toward_its_motion() {
        let mut trident = ThrownTrident::remote(1, Vec3::new(0.5, 100.0, 0.5), 0.0, 10.0);
        trident.velocity = Vec3::new(1.0, 0.0, 1.0);
        trident.remote_tick(&Air, None);
        assert!((trident.yaw - 9.0).abs() < 1e-5, "yaw {}", trident.yaw);
        assert!(
            (trident.pitch - 8.0).abs() < 1e-5,
            "pitch {}",
            trident.pitch
        );
        let mut home = ThrownTrident::remote(2, Vec3::new(0.5, 100.0, 0.5), 0.0, 0.0);
        home.no_physics = true;
        home.velocity = Vec3::new(1.0, 0.0, 1.0);
        home.remote_tick(&Air, None);
        // atan2(-1, -1) = -135: a fifth of the way is -27.
        assert!((home.yaw + 27.0).abs() < 1e-5, "yaw {}", home.yaw);
    }

    /// Solid below y 100.
    struct Solid;
    impl ProjectileWorld for Solid {
        fn clip(&self, _: Vec3, _: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
            None
        }
        fn in_water(&self, _: Aabb) -> bool {
            false
        }
        fn targets(&self) -> &[(Who, Aabb)] {
            &[]
        }
        fn inside_collision(&self, at: Vec3) -> bool {
            at.y < 100.0
        }
    }

    /// `AbstractArrow.tick` runs on a guest too: a position inside a collision
    /// box stops the trident where it is, unless it flies without physics.
    #[test]
    fn a_remote_trident_inside_a_block_stops() {
        let start = Vec3::new(0.5, 99.5, 0.5);
        let mut trident = ThrownTrident::remote(1, start, 0.0, 0.0);
        trident.velocity = Vec3::new(0.5, 0.0, 0.0);
        trident.remote_tick(&Solid, None);
        assert!(trident.in_ground);
        assert_eq!((trident.position, trident.velocity), (start, Vec3::ZERO));

        let mut ghost = ThrownTrident::remote(2, start, 0.0, 0.0);
        ghost.no_physics = true;
        ghost.velocity = Vec3::new(0.5, 0.0, 0.0);
        ghost.remote_tick(&Solid, None);
        assert!(!ghost.in_ground);
        assert_eq!(ghost.position, Vec3::new(1.0, 99.5, 0.5));
    }

    /// `AbstractArrow.onSyncedDataUpdated`: stopping starts a shake of 7, but
    /// not on the first tick, not over a running shake, and not when it was
    /// stuck already.
    #[test]
    fn stopping_inside_a_block_starts_the_shake_once_it_has_flown() {
        let flown = || {
            let mut t = ThrownTrident::remote(1, Vec3::new(0.5, 99.5, 0.5), 0.0, 0.0);
            t.first_tick = false;
            t
        };
        let mut t = flown();
        t.settle_inside_block(true);
        assert_eq!(t.shake, 7);

        let mut first = flown();
        first.first_tick = true;
        first.settle_inside_block(true);
        assert_eq!((first.in_ground, first.shake), (true, 0));

        let mut shaking = flown();
        shaking.shake = 3;
        shaking.settle_inside_block(true);
        assert_eq!(shaking.shake, 3);

        let mut stuck = flown();
        stuck.in_ground = true;
        stuck.settle_inside_block(true);
        assert_eq!(stuck.shake, 0);
    }

    /// A block hit a mod canceled leaves the trident flying while it stays
    /// inside that shape; once out, the next shape stops it again.
    #[test]
    fn a_canceled_hit_flies_on_until_it_leaves_the_shape() {
        let mut t = ThrownTrident::remote(1, Vec3::new(0.5, 99.5, 0.5), 0.0, 0.0);
        t.velocity = Vec3::new(0.5, 0.0, 0.0);
        t.canceled_impact_inside = true;
        t.settle_inside_block(true);
        assert!(!t.in_ground);
        assert_eq!(t.velocity, Vec3::new(0.5, 0.0, 0.0));
        t.settle_inside_block(false);
        assert!(!t.canceled_impact_inside);
        t.settle_inside_block(true);
        assert!(t.in_ground);
    }

    #[test]
    fn remote_trident_waits_for_a_non_self_move_to_leave_its_block() {
        let mut trident = ThrownTrident::remote(1, Vec3::new(0.5, 100.0, 0.5), 0.0, 0.0);
        trident.in_ground = true;
        trident.velocity = Vec3::new(1.0, -2.0, 3.0);
        trident.remote_tick(&Air, None);
        assert!(trident.in_ground);
        assert_eq!(trident.velocity, Vec3::new(1.0, -2.0, 3.0));
    }

    #[test]
    fn piston_move_releases_a_stuck_trident_with_start_falling_velocity() {
        let mut trident = ThrownTrident::remote(1, Vec3::new(0.5, 100.0, 0.5), 0.0, 0.0);
        trident.in_ground = true;
        trident.velocity = Vec3::new(1.0, -2.0, 3.0);
        let mut random = trident.random.clone();
        let expected =
            horizon_client::projectiles::start_falling_velocity(trident.velocity, &mut random);

        trident.move_by_piston(Vec3::new(0.5, 0.0, 0.0));
        assert!(trident.piston_release_bounds().is_some());
        trident.release_after_piston(false);
        assert!(trident.in_ground);
        assert_eq!(trident.velocity, Vec3::new(1.0, -2.0, 3.0));

        trident.release_after_piston(true);
        assert!(!trident.in_ground);
        assert_eq!(trident.velocity, expected);
        assert!(trident.needs_sync);
        assert!(trident.piston_release_bounds().is_none());
    }
}
