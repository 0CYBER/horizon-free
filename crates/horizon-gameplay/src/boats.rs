//! `AbstractBoat`'s state and the rules that need no game: water, land and air status, seats,
//! the bubble-column wobble, saving to NBT and the wood of a boat or raft item.
use horizon_client::{
    entity_rules::EntityRules,
    items::ItemStack,
    mobs::{
        leash::{Holder, Pending},
        riders,
    },
};
use horizon_core::nbt::Tag;
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::EntityRandom,
};
use std::{f32::consts::PI, sync::Arc};

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Counts `Boat::entity_type` calls, for the test that only subscribed blocks build a boat kind.
    #[doc(hidden)]
    pub static TYPE_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    InWater,
    UnderWater,
    UnderFlowingWater,
    OnLand,
    InAir,
}
#[derive(Clone, Debug)]
pub struct Boat {
    /// `Entity.xRot`: kept for `/tp` and `Rotation`; the kind turns only in yaw.
    pub pitch: f32,
    /// When it joined its list (`horizon_client::join_seq`); set by the list.
    pub join: horizon_client::join_seq::JoinSeq,
    pub attachment_lifetime: horizon_mod_runtime::attachments::AttachmentLifetime,
    pub id: u32,
    /// Wood type (`oak`, `bamboo` for the raft).
    pub wood: Arc<str>,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub yaw: f32,
    pub previous_yaw: f32,
    pub delta_rotation: f32,
    pub status: Status,
    pub old_status: Status,
    /// `AbstractBoat.oldStatus` is null until the first tick has set a status.
    pub status_known: bool,
    pub water_level: f64,
    pub land_friction: f32,
    pub out_of_control: f32,
    /// `AbstractBoat.DATA_ID_BUBBLE_TIME` (synched): the ticks of `BUBBLE_TIME`
    /// left before a bubble column lifts or drags the boat; 0 when it is not
    /// over one.
    pub bubble_time: i32,
    /// `AbstractBoat.isAboveBubbleColumn`, set by `onAboveBubbleColumn` and
    /// cleared by `tickBubbleColumn` (the server's).
    pub above_bubble_column: bool,
    /// `AbstractBoat.bubbleColumnDirectionIsDown`.
    pub bubble_down: bool,
    /// The server asked a rider whose client moves it (a player) to get out:
    /// `guest_vehicle_tick` honors it (`Entity.ejectPassengers`).
    pub eject_riders: bool,
    /// `Entity.tickCount`, counted where the wobble is (a client's tick).
    pub tick_count: i32,
    /// `AbstractBoat.bubbleMultiplier`, `bubbleAngle` and `bubbleAngleO`
    /// (a client's).
    pub bubble_multiplier: f32,
    pub bubble_angle: f32,
    pub bubble_angle_old: f32,
    pub hurt_time: i32,
    pub hurt_dir: i32,
    pub damage: f32,
    /// `Entity.remainingFireTicks` (`Fire` in its save): burning while
    /// positive, a fire block's cooldown while negative.
    pub fire_ticks: i32,
    /// `Entity.ticksFrozen` (`TicksFrozen` in its save): powder snow builds
    /// it up (`InsideBlockEffectType.FREEZE`); only a living entity melts it.
    pub ticks_frozen: i32,
    /// `Entity.noGravity` (`NoGravity` in its save).
    pub no_gravity: bool,
    pub paddles: [f32; 2],
    pub paddling: [bool; 2],
    /// Whether this boat has a player controlling its paddles.
    pub paddle_controller: bool,
    /// `getPassengers` in the order a server's last `SetPassengers` listed
    /// them (a server's client); empty where this game seats them itself.
    pub listed_passengers: Vec<u32>,
    /// left, right, up, down
    pub input: [bool; 4],
    pub on_ground: bool,
    /// `Entity.fallDistance`: counted by `AbstractBoat.checkFallDamage`, saved
    /// and loaded (`fall_distance`).
    pub fall_distance: f64,
    /// `Entity.stuckSpeedMultiplier`: set where the boat sits in powder snow
    /// (`PowderSnowBlock.entityInside`) or a cobweb (`WebBlock.entityInside`),
    /// spent by the next move.
    pub stuck: Vec3,
    /// `ChestBoat`: a chest with 27 slots in the back seat.
    pub chest: bool,
    pub items: Vec<Option<ItemStack>>,
    /// `AbstractBoat.leashData`: who holds its lead (`boat_leads.rs`).
    pub leash: Option<Holder>,
    /// The player a `Holder::Player` lead is tied to (their UUID).
    pub leash_player: Option<[i32; 4]>,
    /// A holder read from a save that is not in the world yet.
    pub leash_pending: Option<Pending>,
    /// `LeashData.angularMomentum`.
    pub leash_spin: f64,
    /// `Entity.tickCount`, for the 100 ticks a saved holder is waited for.
    pub age: i32,
    pub random: EntityRandom,
}
/// `ChestBoat.CONTAINER_SIZE`.
pub const CHEST_SLOTS: usize = 27;
/// `AbstractBoat.BUBBLE_TIME`: the ticks over a bubble column before it
/// lifts or drags the boat.
pub const BUBBLE_TIME: i32 = 60;
/// `EntityEvent` 71 and 72, which `AbstractBoat.tickBubbleColumn` broadcasts
/// for a down and an up column.
pub const BUBBLE_DOWN_EVENT: i8 = 71;
pub const BUBBLE_UP_EVENT: i8 = 72;
/// `Entity.load`'s `Fire` (a short; 0 when absent).
pub fn saved_fire(tag: &Tag) -> i32 {
    i32::from(horizon_client::save::number(tag.get("Fire")).unwrap_or(0) as i16)
}
/// `Entity.load`'s `TicksFrozen` (an int; 0 when absent).
pub fn saved_ticks_frozen(tag: &Tag) -> i32 {
    horizon_client::save::number(tag.get("TicksFrozen")).unwrap_or(0) as i32
}
/// `Entity.saveWithoutId`: `TicksFrozen` is written only while positive.
pub fn put_ticks_frozen(map: &mut horizon_core::nbt::Compound, ticks: i32) {
    if ticks > 0 {
        map.insert("TicksFrozen".into(), Tag::Int(ticks));
    } else {
        map.remove(&"TicksFrozen".into());
    }
}
pub const WIDTH: f64 = 1.375;
pub const HEIGHT: f64 = 0.5625;
impl Boat {
    /// `AbstractBoat.tick`: the hurt wobble and the damage wear off, before
    /// `Entity.baseTick` can burn the boat.
    pub fn wear_off_hurt(&mut self) {
        self.hurt_time = (self.hurt_time - 1).max(0);
        self.damage = (self.damage - 1.0).max(0.0);
    }
    /// `AbstractBoat.tick`: passengers lose control after 60 consecutive
    /// underwater ticks; any other status resets the counter.
    pub fn tick_submersion(&mut self) {
        self.out_of_control =
            if matches!(self.status, Status::UnderWater | Status::UnderFlowingWater) {
                self.out_of_control + 1.0
            } else {
                0.0
            };
    }

    pub fn new(id: u32, wood: Arc<str>, position: Vec3, yaw: f32) -> Self {
        Self {
            pitch: 0.0,
            join: Default::default(),
            attachment_lifetime: Default::default(),
            id,
            wood,
            position,
            previous: position,
            velocity: Vec3::ZERO,
            yaw,
            previous_yaw: yaw,
            delta_rotation: 0.0,
            status: Status::InAir,
            old_status: Status::InAir,
            status_known: false,
            water_level: 0.0,
            land_friction: 0.0,
            out_of_control: 0.0,
            bubble_time: 0,
            above_bubble_column: false,
            bubble_down: false,
            eject_riders: false,
            tick_count: 0,
            bubble_multiplier: 0.0,
            bubble_angle: 0.0,
            bubble_angle_old: 0.0,
            hurt_time: 0,
            hurt_dir: 1,
            damage: 0.0,
            fire_ticks: 0,
            ticks_frozen: 0,
            no_gravity: false,
            paddles: [0.0; 2],
            paddling: [false; 2],
            paddle_controller: false,
            listed_passengers: Vec::new(),
            input: [false; 4],
            on_ground: false,
            fall_distance: 0.0,
            stuck: Vec3::ZERO,
            chest: false,
            items: Vec::new(),
            leash: None,
            leash_player: None,
            leash_pending: None,
            leash_spin: 0.0,
            age: 0,
            random: EntityRandom::own(horizon_client::random_source::entity()),
        }
    }
    /// A chest boat or chest raft.
    pub fn with_chest(mut self) -> Self {
        self.chest = true;
        self.items = vec![None; CHEST_SLOTS];
        self
    }
    pub fn bounds(&self) -> Aabb {
        Self::bounds_at(self.position)
    }
    /// The box of a boat whose position is `p`.
    pub fn bounds_at(p: Vec3) -> Aabb {
        let w = WIDTH / 2.0;
        Aabb::new(
            Vec3::new(p.x - w, p.y, p.z - w),
            Vec3::new(p.x + w, p.y + HEIGHT, p.z + w),
        )
    }
    /// `Entity.saveWithoutId` with `AbstractChestBoat`'s items: the loaded
    /// compound `raw` under the live fields.
    pub fn to_nbt(&self, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
        let mut tag = horizon_client::entity_nbt::boat_to_nbt(
            &self.entity_type(),
            self.position,
            self.velocity,
            [self.yaw, self.pitch],
            (self.on_ground, self.fall_distance),
            uuid,
            raw,
            self.chest.then_some(&self.items[..]),
        );
        horizon_client::entity_nbt::write_saved_flags(&mut tag, self.on_ground, self.no_gravity);
        if let Tag::Compound(map) = &mut tag {
            map.insert("Fire".into(), Tag::Short(self.fire_ticks as i16));
            put_ticks_frozen(map, self.ticks_frozen);
        }
        tag
    }
    /// The start of `Entity.move`: a boat made stuck since its last move goes its
    /// velocity times the multiplier, and its velocity and the multiplier are spent.
    pub fn take_movement(&mut self) -> Vec3 {
        if self.stuck.length_squared() <= 1.0e-7 {
            return self.velocity;
        }
        let movement = self.velocity.multiply(self.stuck);
        self.stuck = Vec3::ZERO;
        self.velocity = Vec3::ZERO;
        movement
    }
    /// `isUnderWater`; it stands in for `canAddPassenger`'s
    /// `isEyeInFluid(WATER)` too.
    pub fn under_water(&self) -> bool {
        matches!(self.status, Status::UnderWater | Status::UnderFlowingWater)
    }
    /// `Entity.isInLiquid`, for `Leashable.angularFriction`.
    pub fn in_liquid(&self) -> bool {
        matches!(
            self.status,
            Status::InWater | Status::UnderWater | Status::UnderFlowingWater
        )
    }
    /// `getMaxPassengers`: two, one beside a chest.
    pub fn max_passengers(&self) -> usize {
        if self.chest { 1 } else { 2 }
    }
    /// `rideHeight`: a third of a boat's height, 8/9 of a raft's.
    pub fn ride_height(&self) -> f64 {
        let height = HEIGHT as f32;
        f64::from(if &*self.wood == "bamboo" {
            height * 0.888_888_9
        } else {
            height / 3.0
        })
    }
    /// Where the passenger at `index` of `count` stands, at the boat's current
    /// (or, with `previous`, last) place: `getPassengerAttachmentPoint`
    /// turned with the boat, less the passenger's `rider_offset`.
    pub fn seat(
        &self,
        index: usize,
        count: usize,
        animal: bool,
        rider_offset: f64,
        previous: bool,
    ) -> Vec3 {
        let mut z = if self.chest { 0.15f32 } else { 0.0 };
        if count > 1 {
            z = if index == 0 { 0.2 } else { -0.6 };
            if animal {
                z += 0.2;
            }
        }
        let (at, yaw) = if previous {
            (self.previous, self.previous_yaw)
        } else {
            (self.position, self.yaw)
        };
        riders::seat(
            [0.0, self.ride_height(), f64::from(z)],
            rider_offset,
            at,
            yaw,
        )
    }
    /// `AbstractBoat.getBubbleAngle`: the wobble, in degrees, `a` of the way
    /// from the last tick's to this one's.
    pub fn bubble_angle(&self, a: f32) -> f32 {
        self.bubble_angle_old + a * (self.bubble_angle - self.bubble_angle_old)
    }
    /// `AbstractBoat.tickBubbleColumn`: on a client, the wobble grows by 0.05 a
    /// tick while the synched bubble time runs and fades by 0.1 otherwise.
    pub fn tick_bubble_wobble(&mut self) {
        self.tick_count += 1;
        if self.bubble_time > 0 {
            self.bubble_multiplier += 0.05;
        } else {
            self.bubble_multiplier -= 0.1;
        }
        self.bubble_multiplier = self.bubble_multiplier.clamp(0.0, 1.0);
        self.bubble_angle_old = self.bubble_angle;
        self.bubble_angle =
            10.0 * (0.5 * f64::from(self.tick_count)).sin() as f32 * self.bubble_multiplier;
    }
    /// `AbstractBoat.tickBubbleColumn`'s countdown on the server: the bubble
    /// time drops while the boat stays over a column and resets when it leaves.
    /// `Some(down)` on the tick it runs out: the boat is thrown up (`false`) or
    /// pulled down (`true`).
    pub fn tick_bubble_countdown(&mut self) -> Option<bool> {
        if !self.above_bubble_column {
            self.bubble_time = 0;
        }
        if self.bubble_time <= 0 {
            return None;
        }
        self.bubble_time -= 1;
        let diff = BUBBLE_TIME - self.bubble_time - 1;
        let fired = (diff > 0 && self.bubble_time == 0).then_some(self.bubble_down);
        self.above_bubble_column = false;
        fired
    }
    /// `getRowingTime`.
    pub fn paddle_state(&self, side: usize) -> bool {
        self.paddling[side] && self.paddle_controller
    }
    pub fn rowing_time(&self, side: usize, a: f32) -> f32 {
        if self.paddle_state(side) {
            let current = self.paddles[side];
            let previous = current - PI / 8.0;
            previous + a.clamp(0.0, 1.0) * (current - previous)
        } else {
            0.0
        }
    }
}
impl Boat {
    /// The vanilla entity type id (`oak_boat`, `bamboo_chest_raft`), also
    /// the item it drops.
    pub fn entity_type(&self) -> String {
        #[cfg(any(test, feature = "test-support"))]
        TYPE_BUILDS.with(|n| n.set(n.get() + 1));
        let chest = if self.chest { "chest_" } else { "" };
        if &*self.wood == "bamboo" {
            format!("minecraft:bamboo_{chest}raft")
        } else {
            format!("minecraft:{}_{chest}boat", self.wood)
        }
    }
}
/// The wood of a boat or chest boat item or entity type, and whether it
/// carries a chest.
pub fn any_boat_wood(item: &str) -> Option<(&str, bool)> {
    let name = item.strip_prefix("minecraft:")?;
    if let Some(wood) = name
        .strip_suffix("_chest_boat")
        .or_else(|| name.strip_suffix("_chest_raft"))
    {
        return Some((wood, true));
    }
    boat_wood(item).map(|w| (w, false))
}
pub fn boat_wood(item: &str) -> Option<&str> {
    let name = item.strip_prefix("minecraft:")?;
    name.strip_suffix("_boat")
        .or_else(|| name.strip_suffix("_raft"))
        .filter(|w| !w.ends_with("_chest"))
}
impl EntityRules for Boat {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// `AbstractBoat.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        0.04
    }
}
horizon_client::impl_joined!(Boat);
horizon_mod_runtime::impl_attachment_lifetime!(Boat);
