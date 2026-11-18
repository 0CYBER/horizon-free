//! Firework rocket state (`FireworkRocketEntity`): the fields, loading from and saving to NBT, the
//! 1.15 horizontal acceleration and 0.04 lift of one move, and the item's default flight. Launching,
//! blasts and the client explosion stay in `horizon-client`.
use horizon_client::{
    collisions::SceneCollisions, entity_nbt::RocketNbt, items::ItemStack, mobs::Who,
    particles::FireworkExplosion,
};
use horizon_core::geometry::{Aabb, Vec3};

pub struct Rocket {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: horizon_mod_runtime::attachments::AttachmentLifetime,
    /// `Entity.yRot` and `xRot` (`/tp`, `/data get ... Rotation`).
    pub rotation: [f32; 2],
    /// When it joined its list (`horizon_client::join_seq`); set by the list.
    pub join: horizon_client::join_seq::JoinSeq,
    /// Its entity id: from the entity counter on the server, the server's on
    /// a client.
    pub id: u32,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub life: i32,
    pub lifetime: i32,
    pub explosions: Vec<FireworkExplosion>,
    /// Boosting the player (attachedToEntity).
    pub attached: bool,
    /// On a server, the guest (by connection) who launched it, or whom it
    /// boosts: `owner`'s `Who::Player` then means them, not the host.
    pub guest: Option<u32>,
    /// `Projectile.getOwner`: the player who launched it, credited with
    /// what its blast hurts.
    pub owner: Option<Who>,
    /// `Projectile.getOwner` when `owner` can't name it: the UUID of an owner
    /// that is neither a player nor a mob.
    pub owner_uuid: Option<[i32; 4]>,
    /// `Projectile.leftOwner`: until then it can't hit its owner.
    pub left_owner: bool,
    /// `DATA_ATTACHED_TO_TARGET`: the server entity the rocket follows;
    /// boosting happens when it is the local player.
    pub attached_to: Option<u32>,
    /// `DATA_SHOT_AT_ANGLE` (dispensers, crossbows): constant velocity.
    pub angled: bool,
    /// `DATA_ID_FIREWORKS_ITEM`: the stack it was launched from.
    pub item: ItemStack,
    /// `Entity.random`: what a mod's `Projectile.deflect` draws from.
    pub random: horizon_core::random::EntityRandom,
    /// `Entity.onGround`: whether its last move was stopped going down (or
    /// the one it was loaded with, until it moves).
    pub on_ground: bool,
    /// `Projectile.hasBeenShot`: its first tick posts `projectile_shoot`.
    pub has_been_shot: bool,
    /// `Entity.isSilent`: no launch sound.
    pub silent: bool,
    /// `Entity.fallDistance`.
    pub fall_distance: f64,
    /// `Entity.stuckSpeedMultiplier`: a cobweb or powder snow it went through
    /// slows its next move.
    pub stuck: Vec3,
    /// `Entity.wasTouchingWater`: entering water splashes.
    pub was_touching_water: bool,
    /// `Entity.firstTick`: no splash on the tick it appears in water.
    pub first_tick: bool,
}
impl Rocket {
    /// A rocket a server sent (`handleAddEntity`): placed and moved by it.
    pub fn remote(id: u32, position: Vec3, velocity: Vec3) -> Self {
        let item = ItemStack::new("minecraft:firework_rocket", 1);
        let (_, explosions) = fireworks_of(&item);
        Self {
            attachment_lifetime: Default::default(),
            rotation: [0.0, 0.0],
            join: Default::default(),
            id,
            position,
            previous: position,
            velocity,
            life: 0,
            lifetime: i32::MAX,
            explosions,
            attached: false,
            guest: None,
            owner: None,
            owner_uuid: None,
            left_owner: false,
            attached_to: None,
            angled: false,
            item,
            random: horizon_client::projectiles::unique_random(position),
            on_ground: false,
            // The server posts the shot.
            has_been_shot: true,
            silent: false,
            fall_distance: 0.0,
            stuck: Vec3::ZERO,
            was_touching_water: false,
            first_tick: true,
        }
    }
    /// `Projectile.setOwner`; the UUID is kept only for an owner `who` can't name.
    pub fn set_owner(&mut self, who: Option<Who>, uuid: Option<[i32; 4]>, guest: Option<u32>) {
        (self.owner, self.guest) = (who, guest);
        self.owner_uuid = uuid.filter(|_| who.is_none());
    }
    /// `FireworkRocketEntity.addAdditionalSaveData`; the owner when it is
    /// `player`, or an entity that is neither a player nor a mob.
    pub fn to_nbt(
        &self,
        uuid: [i32; 4],
        player: Option<[i32; 4]>,
        raw: Option<&horizon_core::nbt::Tag>,
    ) -> horizon_core::nbt::Tag {
        horizon_client::entity_nbt::firework_rocket_to_nbt(
            &horizon_client::entity_nbt::RocketSave {
                rotation: self.rotation,
                position: self.position,
                motion: self.velocity,
                life: self.life,
                lifetime: self.lifetime,
                item: &self.item,
                angled: self.angled,
                owner: if matches!(self.owner, Some(Who::Player)) {
                    player
                } else {
                    self.owner_uuid
                },
                left_owner: self.left_owner,
                on_ground: self.on_ground,
                has_been_shot: self.has_been_shot,
                fall_distance: self.fall_distance,
            },
            uuid,
            raw,
        )
    }
    /// A rocket loaded from its NBT (`/summon`, a saved chunk).
    pub fn from_nbt(r: RocketNbt) -> Self {
        let item = r
            .item
            .unwrap_or_else(|| ItemStack::new("minecraft:firework_rocket", 1));
        let (_, explosions) = fireworks_of(&item);
        Self {
            attachment_lifetime: Default::default(),
            rotation: r.rotation,
            join: Default::default(),
            id: 0,
            position: r.position,
            previous: r.position,
            velocity: r.motion,
            life: r.life,
            lifetime: r.lifetime,
            explosions,
            attached: false,
            guest: None,
            owner: r.owner,
            owner_uuid: r.owner_uuid.filter(|_| r.owner.is_none()),
            left_owner: r.left_owner,
            attached_to: None,
            angled: r.angled,
            item,
            random: horizon_client::projectiles::unique_random(r.position),
            on_ground: r.on_ground,
            has_been_shot: r.has_been_shot,
            silent: r.silent,
            fall_distance: r.fall_distance,
            stuck: Vec3::ZERO,
            was_touching_water: false,
            first_tick: true,
        }
    }
    /// `Entity.load` over a live rocket (`/data merge`, `/data modify`): what
    /// [`Self::from_nbt`] reads replaces its own; it keeps its id and what it
    /// boosts.
    pub fn load_nbt(&mut self, r: RocketNbt) {
        let loaded = Self::from_nbt(r);
        self.rotation = loaded.rotation;
        self.has_been_shot = loaded.has_been_shot;
        self.silent = loaded.silent;
        self.position = loaded.position;
        self.previous = loaded.position;
        self.velocity = loaded.velocity;
        self.life = loaded.life;
        self.lifetime = loaded.lifetime;
        self.explosions = loaded.explosions;
        self.item = loaded.item;
        self.angled = loaded.angled;
        self.left_owner = loaded.left_owner;
        self.on_ground = loaded.on_ground;
        self.fall_distance = loaded.fall_distance;
    }
    /// One tick of a server's client: the rocket flies on by itself between
    /// the server's updates (a rocket that follows an entity is placed by the
    /// caller).
    pub fn remote_step(
        &mut self,
        scene: &mut SceneCollisions,
        context: &horizon_world::native_load_context::LoopContext,
    ) {
        self.previous = self.position;
        if self.attached_to.is_none() {
            let _ = self.free_flight(scene, context);
        }
        self.life += 1;
    }
    /// The end of `Projectile.shoot`: yaw and pitch are set straight from the
    /// shot's motion (the old rotation follows, as `rotation` is the old one
    /// on the next tick).
    pub fn turn_to_shot(&mut self) {
        let v = self.velocity;
        let horizontal = (v.x * v.x + v.z * v.z).sqrt();
        let atan2 = horizon_core::geometry::trig::atan2_degrees;
        self.rotation = [atan2(v.x, v.z), atan2(v.y, horizontal)];
    }
    /// `Projectile.updateRotation`: a fifth of the way from `old` (yaw,
    /// pitch) toward its motion's direction, the short way round.
    pub fn update_rotation(&mut self, old: [f32; 2]) {
        let v = self.velocity;
        let horizontal = (v.x * v.x + v.z * v.z).sqrt();
        let atan2 = horizon_core::geometry::trig::atan2_degrees;
        self.rotation = [
            horizon_client::arrows::lerp_rotation(old[0], atan2(v.x, v.z)),
            horizon_client::arrows::lerp_rotation(old[1], atan2(v.y, horizontal)),
        ];
    }
    /// The move of a rocket attached to no entity: sped up and rising, or an
    /// angled one's own motion.
    pub fn next_motion(&self) -> Vec3 {
        if self.angled {
            self.velocity
        } else {
            Vec3::new(
                self.velocity.x * 1.15,
                self.velocity.y + 0.04,
                self.velocity.z * 1.15,
            )
        }
    }
    /// `EntityType.FIREWORK_ROCKET`: 0.25 x 0.25.
    pub fn bounds(&self) -> Aabb {
        Self::bounds_at(self.position)
    }
    /// [`Self::bounds`] at `position`.
    pub fn bounds_at(position: Vec3) -> Aabb {
        let h = 0.125;
        Aabb::new(
            Vec3::new(position.x - h, position.y, position.z - h),
            Vec3::new(position.x + h, position.y + 2.0 * h, position.z + h),
        )
    }
    /// `FireworkRocketEntity.tick` for a rocket attached to no entity: speed
    /// up (an angled one keeps its motion) and rise, then move (`Entity.move`:
    /// a cobweb or powder snow scales this move only). Returns where it moved
    /// from, the move it asked for and the one it made.
    pub fn free_flight(
        &mut self,
        scene: &mut SceneCollisions,
        context: &horizon_world::native_load_context::LoopContext,
    ) -> Option<(Vec3, Vec3, Vec3)> {
        let bounds = self.bounds();
        let motion = self.next_motion();
        let mut wanted = motion;
        if self.stuck.length_squared() > 1.0e-7 {
            wanted = Vec3::new(
                wanted.x * self.stuck.x,
                wanted.y * self.stuck.y,
                wanted.z * self.stuck.z,
            );
            self.stuck = Vec3::ZERO;
        }
        let from = self.position;
        let mut gate = horizon_client::move_gate::armed(
            context,
            self.id,
            "minecraft:firework_rocket",
            horizon_world::entity_move::MoverType::SelfMovement,
            wanted,
        );
        let wanted = gate.delta();
        let Ok(moved) =
            horizon_client::move_gate::scene_resolve(&mut gate, scene, bounds, wanted, 0.0, false)
        else {
            return None;
        };
        let horizontal = (moved.x - wanted.x).abs() > 1e-7 || (moved.z - wanted.z).abs() > 1e-7;
        // Entity.move: on the ground when a move down was stopped.
        self.on_ground = wanted.y < 0.0 && moved.y != wanted.y;
        self.position = Vec3::new(
            self.position.x + moved.x,
            self.position.y + moved.y,
            self.position.z + moved.z,
        );
        // setDeltaMovement(movement2): the motion before the move's scaling.
        self.velocity = if horizontal {
            Vec3::new(self.velocity.x, motion.y, self.velocity.z)
        } else {
            motion
        };
        Some((from, wanted, moved))
    }
    /// Position and motion (the dispenser parity fixture).
    pub fn motion(&self) -> (Vec3, Vec3) {
        (self.position, self.velocity)
    }
}
pub fn fireworks_of(stack: &ItemStack) -> (u8, Vec<FireworkExplosion>) {
    // The item's default component: flight 1, no explosions.
    stack
        .data()
        .and_then(|d| d.fireworks.clone())
        .unwrap_or((1, Vec::new()))
}
horizon_client::impl_joined!(Rocket);
horizon_mod_runtime::impl_attachment_lifetime!(Rocket);
impl horizon_client::entity_rules::EntityRules for Rocket {}
