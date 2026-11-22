//! Minecarts (`AbstractMinecart` with `OldMinecartBehavior`): the cart's state, its kinds, the
//! rail shapes, the coarse index of carts and mobs for pushes, the pure rules around them, and
//! the save and load of a cart. The parts that need `Game` stay in horizon-client.
use horizon_client::fast_hash::FxHashSet;
use horizon_client::join_seq::{JoinList, ListStamp};
use horizon_client::mobs::{Mob, seen_grid::SeenGrid};
use horizon_client::{
    items::{ItemStack, Items},
    mobs::{Part, Species},
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::EntityRandom,
};
use horizon_world::registry::BlockStateId;
use std::{collections::HashMap, sync::Arc};

pub mod seat;
pub use seat::*;

/// `Entity.load`'s `TicksFrozen` (an int; 0 when absent).
fn saved_ticks_frozen(tag: &horizon_core::nbt::Tag) -> i32 {
    horizon_client::save::number(tag.get("TicksFrozen")).unwrap_or(0) as i32
}
/// `Entity.saveWithoutId`: `TicksFrozen` is written only while positive.
fn put_ticks_frozen(map: &mut horizon_core::nbt::Compound, ticks: i32) {
    if ticks > 0 {
        map.insert("TicksFrozen".into(), horizon_core::nbt::Tag::Int(ticks));
    } else {
        map.remove(&"TicksFrozen".into());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CartKind {
    Rideable,
    Chest,
    Hopper,
    Furnace,
    Tnt,
    /// `MinecartCommandBlock`: its `BaseCommandBlock` is `Minecart::command`,
    /// run by a powered activator rail (`game/administration/command_minecarts.rs`).
    CommandBlock,
    /// `MinecartSpawner`: a `BaseSpawner` that ticks where the cart is
    /// (`Minecart::spawner`). Only `/summon` makes one: there is no item.
    Spawner,
}
impl CartKind {
    pub fn from_id(id: &str) -> Option<Self> {
        Some(match id.strip_prefix("minecraft:")? {
            "minecart" => Self::Rideable,
            "chest_minecart" => Self::Chest,
            "hopper_minecart" => Self::Hopper,
            "furnace_minecart" => Self::Furnace,
            "tnt_minecart" => Self::Tnt,
            "command_block_minecart" => Self::CommandBlock,
            "spawner_minecart" => Self::Spawner,
            _ => return None,
        })
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Rideable => "minecraft:minecart",
            Self::Chest => "minecraft:chest_minecart",
            Self::Hopper => "minecraft:hopper_minecart",
            Self::Furnace => "minecraft:furnace_minecart",
            Self::Tnt => "minecraft:tnt_minecart",
            Self::CommandBlock => "minecraft:command_block_minecart",
            Self::Spawner => "minecraft:spawner_minecart",
        }
    }
    /// `AbstractMinecart.getDropItem` and `getPickResult`: the item a cart drops and
    /// is picked as (a spawner minecart's is a plain minecart: there is no item of its own).
    pub fn item(self) -> &'static str {
        match self {
            Self::Spawner => "minecraft:minecart",
            kind => kind.id(),
        }
    }
    pub fn slots(self) -> usize {
        match self {
            Self::Chest => 27,
            Self::Hopper => 5,
            _ => 0,
        }
    }
    /// Whether this cart's `interact` passes on the client, so that
    /// `Minecraft.startUseItem` uses the held item instead: `MinecartTNT` has
    /// none of its own (`Entity.interact`), `MinecartCommandBlock.interact`
    /// needs a game master, and `Minecart.interact` a player not sneaking at
    /// an empty cart.
    pub fn interact_passes(self, sneaking: bool, ridden: bool, game_master: bool) -> bool {
        match self {
            Self::Tnt | Self::Spawner => true,
            Self::CommandBlock => !game_master,
            Self::Rideable => sneaking || ridden,
            Self::Chest | Self::Hopper | Self::Furnace => false,
        }
    }
}
/// The boxes of the other carts that meet `area`, taken in list order when a cart's tick began: the
/// carts do not move while one of them ticks, so the boxes its moves meet inside `area` are in here.
#[derive(Clone, Debug)]
pub struct NearCarts {
    pub area: Aabb,
    pub boxes: Vec<Aabb>,
}
/// How far past a cart's box the boxes are taken when its tick begins (a longer move asks the list).
pub const NEAR_CARTS_REACH: f64 = 8.0;
impl NearCarts {
    /// The boxes that meet `near`, in list order; None when `near` is not inside the area taken.
    pub fn meeting(&self, near: Aabb) -> Option<Vec<Aabb>> {
        let inside = [
            horizon_core::geometry::Axis::X,
            horizon_core::geometry::Axis::Y,
            horizon_core::geometry::Axis::Z,
        ]
        .into_iter()
        .all(|a| self.area.min(a) <= near.min(a) && near.max(a) <= self.area.max(a));
        inside.then(|| {
            self.boxes
                .iter()
                .copied()
                .filter(|b| b.intersects(near))
                .collect()
        })
    }
}

#[derive(Clone, Debug)]
pub struct Minecart {
    pub attachment_lifetime: horizon_mod_runtime::attachments::AttachmentLifetime,
    /// When it joined its list (`horizon_client::join_seq`); set by the list.
    pub join: horizon_client::join_seq::JoinSeq,
    pub id: u32,
    pub kind: CartKind,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub yaw: f32,
    pub previous_yaw: f32,
    pub pitch: f32,
    pub previous_pitch: f32,
    /// Minecart's client-only accumulated turn and the passenger's eased turn.
    pub rotation_offset: f32,
    pub player_rotation_offset: f32,
    pub passenger_yaw_delta: f32,
    /// Experimental track steps, drained by the server entity tracker.
    pub lerp_steps: Vec<horizon_protocol::packets::play::MinecartStep>,
    pub flipped: bool,
    /// `AbstractMinecart.onRails`: transitions into rails need a position snap.
    pub on_rails: bool,
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub fall_distance: f64,
    /// The squared speed `MinecartTNT.causeFallDamage` blows up with, set by
    /// a landing of three blocks or more and taken by the cart's next tick
    /// step (vanilla explodes inside the move itself).
    pub fall_blast: Option<f64>,
    /// `Entity.stuckSpeedMultiplier`: set by a cobweb the cart's box is in,
    /// scaled into its next move.
    pub stuck: Vec3,
    pub hurt_time: i32,
    pub hurt_dir: i32,
    pub damage: f32,
    /// `Entity.noGravity` (`NoGravity` in its save).
    pub no_gravity: bool,
    /// `Entity.ticksFrozen` (`TicksFrozen` in its save): powder snow builds
    /// it up (`InsideBlockEffectType.FREEZE`), a cart never melts it.
    pub ticks_frozen: i32,
    /// The cactus hits its `applyEffectsFromBlocks` met this tick, taken once it is stored.
    pub cactus_hits: u32,
    pub items: Vec<Option<ItemStack>>,
    /// Furnace minecarts: fuel ticks and `MinecartFurnace.push`. Its y part
    /// stays between ticks (it sets the push's length), only saving drops it.
    pub fuel: i32,
    pub push: Vec3,
    /// TNT minecarts: ticks until the explosion, -1 when not primed.
    pub fuse: i32,
    /// TNT minecarts: `explosionPowerBase` and `explosionSpeedFactor`, the
    /// blast's power at rest and its gain with the cart's speed.
    pub explosion_power: f32,
    pub explosion_speed_factor: f32,
    /// Hopper minecarts: an unpowered activator rail enables them.
    pub enabled: bool,
    /// Chest minecarts: the loot table rolled into `items` on load, saved
    /// again while the items are still that roll.
    pub loot: Option<horizon_client::save::PendingContainerLoot>,
    /// `Entity.firstTick`: not ticked yet (saved as `HasTicked`).
    pub first_tick: bool,
    /// `Entity.isInWater` (`wasTouchingWater`): `AbstractMinecart.tick`
    /// updates it after the move (`updateFluidInteraction`), so the move
    /// goes by where the cart was a tick before, and a new cart's first
    /// move is a dry one.
    pub in_water: bool,
    /// Command block minecarts: the cart's `BaseCommandBlock` (command,
    /// success count, last output, output tracking). The block-only flags of
    /// `CommandBlock` stay unset.
    pub command: horizon_client::command_block::CommandBlock,
    /// `Entity.tickCount`, which `MinecartCommandBlock.activateMinecart`
    /// measures its `ACTIVATION_DELAY` against.
    pub tick_count: i32,
    /// `MinecartCommandBlock.lastActivated`.
    pub last_activated: i32,
    /// An activator rail asked for the command during this tick; it runs once
    /// the cart is back in the level (`command_minecarts.rs`).
    pub command_due: bool,
    /// Spawner minecarts: the cart's `BaseSpawner` (`Delay`, `SpawnData`,
    /// `SpawnPotentials` and the rest as a spawner block saves them).
    pub spawner: Option<Box<horizon_client::mobs::spawner::Spawner>>,
    /// The synched `DATA_ID_COMMAND_NAME` and `DATA_ID_LAST_OUTPUT`, which
    /// follow the command block only at `onUpdated`.
    pub synced_command: String,
    pub synced_output: String,
    /// `AbstractMinecart.customDisplayBlockState` (`DisplayState`): the block
    /// state's NBT form, resolved against the registry where it draws.
    pub custom_display: Option<horizon_core::nbt::Tag>,
    /// `AbstractMinecart.customDisplayOffset` (`DisplayOffset`), in sixteenths.
    pub custom_offset: Option<i32>,
    pub random: EntityRandom,
    /// Collision boxes cached during this cart's movement.
    pub near_carts: Option<Box<NearCarts>>,
}
/// `EntityType.MINECART`'s `sized(0.98F, 0.7F)`: float dimensions, widened.
pub const WIDTH: f64 = 0.98_f32 as f64;
pub const HEIGHT: f64 = 0.7_f32 as f64;
/// `EntityEvent.TNT_PRIME`. `MinecartTNT.primeFuse` broadcasts it;
/// `MinecartTNT.handleEntityEvent` answers by calling `primeFuse`.
pub const TNT_PRIME_EVENT: i8 = 70;
/// `AbstractMinecart.getDefaultDisplayOffset` (6), in sixteenths of a block, as
/// `MinecartChest` (8) and `MinecartHopper` (1) override it.
pub fn default_display_offset(kind: CartKind) -> f64 {
    match kind {
        CartKind::Chest => 8.0,
        CartKind::Hopper => 1.0,
        CartKind::Rideable
        | CartKind::Furnace
        | CartKind::Tnt
        | CartKind::CommandBlock
        | CartKind::Spawner => 6.0,
    }
}
/// `AbstractMinecart.getDisplayOffset`: the saved `DisplayOffset`, else the default.
pub fn display_offset(c: &Minecart) -> f64 {
    c.custom_offset
        .map_or_else(|| default_display_offset(c.kind), f64::from)
}

/// World-space envelope of the cart model and its optional displayed block,
/// based on the same root position and display offset used by the renderer.
pub fn cart_render_bounds(position: Vec3, offset: f64, has_display: bool) -> Aabb {
    let (bottom, top) = if has_display {
        let display_shift = 0.75 * (offset - 8.0) / 16.0;
        (
            (position.y - 0.5).min(position.y + 0.375 - 0.4 + display_shift),
            (position.y + 0.9).max(position.y + 0.375 + 0.4 + display_shift + 0.9),
        )
    } else {
        (position.y - 0.5, position.y + 0.9)
    };
    Aabb::new(
        Vec3::new(position.x - 1.0, bottom, position.z - 1.0),
        Vec3::new(position.x + 1.0, top, position.z + 1.0),
    )
}
/// A display state that is air shows nothing.
pub fn shown_unless_air(
    state: Option<BlockStateId>,
    is_air: impl Fn(BlockStateId) -> bool,
) -> Option<BlockStateId> {
    state.filter(|&s| !is_air(s))
}
/// `readAdditionalSaveData`'s `input.read("DisplayState", CODEC)` is empty
/// when the state does not decode: the cart forgets it (and no longer saves it).
pub fn forget_unreadable_display(
    c: &mut Minecart,
    resolves: impl Fn(&horizon_core::nbt::Tag) -> bool,
) {
    if c.custom_display.as_ref().is_some_and(|t| !resolves(t)) {
        c.custom_display = None;
    }
}

/// The carts' and mobs' boxes for one tick's cart pushes (`push_from_cart`,
/// `pickup_from_cart`), so a cart reads its neighbours instead of every cart and
/// every mob of the level. Both answer in slot order, as the scans did.
///
/// The cart grid follows each ticked cart ([`CartIndex::moved`]); it is
/// rebuilt once the list's length changed (a cart gone or added), and dropped
/// after a command, which can move or replace any cart
/// ([`CartIndex::forget_carts`]). The mob grid holds until the mob
/// list's next write ([`ListStamp`]). A native mod's hook can move either kind
/// unseen, so with mods loaded both scans stay.
#[derive(Default)]
pub struct CartIndex {
    /// The tick pass over the carts shares a grid for the movement snapshots.
    pub pass: bool,
    pub carts: Option<(SeenGrid, Vec<u32>)>,
    /// The grid, and the carts a mob rides.
    pub mobs: Option<(ListStamp, SeenGrid, FxHashSet<u32>)>,
    pub near: Vec<u32>,
}
impl CartIndex {
    pub fn usable() -> bool {
        mod_api::__private::mods().is_empty()
    }
    /// The cart in slot `i` has these bounds now.
    pub fn moved(&mut self, i: usize, bounds: Aabb) {
        if let Some((grid, ids)) = &mut self.carts
            && i < ids.len()
        {
            grid.update(i, bounds);
        }
    }
    pub fn forget_carts(&mut self) {
        self.carts = None;
    }
    /// The slots of the carts whose box may meet `area`, for the cart in slot
    /// `i` (just moved); false: scan them all.
    pub fn carts_near(
        &mut self,
        carts: &[Minecart],
        i: usize,
        area: Aabb,
        out: &mut Vec<u32>,
    ) -> bool {
        if !Self::usable() {
            return false;
        }
        // Removing a cart and adding another at the end keeps the length.
        let fresh = |ids: &[u32]| {
            ids.len() == carts.len()
                && ids[i] == carts[i].id
                && ids.last() == carts.last().map(|c| &c.id)
        };
        if !self.carts.as_ref().is_some_and(|(_, ids)| fresh(ids)) {
            self.carts = Some((
                SeenGrid::new(carts.iter().map(Minecart::bounds)),
                carts.iter().map(|c| c.id).collect(),
            ));
        }
        let Some((grid, ids)) = &mut self.carts else {
            return false;
        };
        grid.update(i, carts[i].bounds());
        if !grid.candidates(area, out) {
            return false;
        }
        if out.iter().any(|&j| ids[j as usize] != carts[j as usize].id) {
            // A cart was replaced in place: start over next time.
            self.carts = None;
            return false;
        }
        true
    }
    /// The boxes of the other carts that meet `area`, in list order, for the cart in slot `i`;
    /// None: the cart asks the list itself.
    pub fn near_snapshot(&mut self, carts: &[Minecart], i: usize, area: Aabb) -> Option<NearCarts> {
        if !self.pass {
            return None;
        }
        let mut slots = Vec::new();
        if !self.carts_near(carts, i, area, &mut slots) {
            return None;
        }
        let id = carts[i].id;
        let boxes = slots
            .iter()
            .map(|&j| &carts[j as usize])
            .filter(|o| o.id != id)
            .map(Minecart::bounds)
            .filter(|b| b.intersects(area))
            .collect();
        Some(NearCarts { area, boxes })
    }
    /// The slots of the mobs whose box may meet `area` into `out`, and
    /// whether a mob rides cart `cart`; None: scan them all.
    pub fn mobs_near(
        &mut self,
        mobs: &JoinList<Mob>,
        cart: u32,
        area: Aabb,
        out: &mut Vec<u32>,
    ) -> Option<bool> {
        if !Self::usable() {
            return None;
        }
        if !self
            .mobs
            .as_ref()
            .is_some_and(|(stamp, ..)| *stamp == mobs.stamp())
        {
            let mut mounted = FxHashSet::default();
            let grid = SeenGrid::new(mobs.iter().map(|m| {
                if let Some(v) = m.extra.mount {
                    mounted.insert(v);
                }
                m.bounds()
            }));
            self.mobs = Some((mobs.stamp(), grid, mounted));
        }
        let (_, grid, mounted) = self.mobs.as_ref()?;
        grid.candidates(area, out).then(|| mounted.contains(&cart))
    }
}
/// Where the cart `id` is in `carts`: at `hint` (where it was a moment ago,
/// the usual case while carts tick in order) or found by a scan. A world
/// with hundreds of mineshaft chest carts asked this of every cart several
/// times a tick.
pub fn cart_at(carts: &[Minecart], id: u32, hint: usize) -> Option<usize> {
    if carts.get(hint).is_some_and(|c| c.id == id) {
        return Some(hint);
    }
    carts.iter().position(|c| c.id == id)
}
/// `AbstractMinecart.push(entity)` for a non-cart `entity` at `feet`:
/// the push the cart loses (the entity gets a quarter of it back).
pub fn cart_push_from(feet: Vec3, cart: Vec3) -> Option<(f64, f64)> {
    let (xa, za) = (feet.x - cart.x, feet.z - cart.z);
    let dd = xa * xa + za * za;
    if dd < f64::from(1.0e-4f32) {
        return None;
    }
    let d = dd.sqrt();
    let pow = (1.0 / d).min(1.0);
    let k = f64::from(0.1f32) * 0.5;
    Some((xa / d * pow * k, za / d * pow * k))
}
/// `MinecartFurnace.calculateNewPushAlong`: project the push onto movement.
pub fn push_along(push: Vec3, movement: Vec3) -> Vec3 {
    if push.horizontal_distance_squared() > 1.0e-4 && movement.horizontal_distance_squared() > 0.001
    {
        push.projected_on(movement).normalize().scale(push.length())
    } else {
        push
    }
}
/// The minecart's passenger attachment minus Player.DEFAULT_VEHICLE_ATTACHMENT.
pub const SEAT: f64 = PASSENGER_Y - 0.6;
/// The placeholder container key under which an open cart's items live.
pub const CART_KEY_Y: i32 = horizon_client::lan::menu::VEHICLE_KEY_Y;

/// `OldMinecartBehavior.pushAndPickupEntities`: living entities the cart
/// offers `startRiding` instead of `Entity.push`. Players click to board;
/// iron golems always push; a cart that already has a rider is `isVehicle`.
pub fn pickup_instead_of_push(species: Species, is_passenger: bool, cart_is_vehicle: bool) -> bool {
    !cart_is_vehicle && !is_passenger && species != Species::IronGolem
}

/// `EntitySelector.pushableBy` asks `isPushable`: `Shulker` takes no push
/// (`Mob::pushable` leaves it out) but still counts, so a cart offers it
/// `startRiding` like any other living entity.
pub fn offered_pickup(pushable: bool, species: Species, alive: bool) -> bool {
    pushable || species == Species::Shulker && alive
}

/// `Shulker.push(Entity)` is empty, so a shulker that is not picked up (it
/// already rides something) neither moves nor slows the cart.
pub fn takes_push_from_cart(species: Species) -> bool {
    species != Species::Shulker
}

/// `Entity.canRide` for a cart: the boarding cooldown, and a warden or the
/// wither never rides (`canRide` false).
pub fn can_ride_cart(species: Species, boarding_cooldown: i32) -> bool {
    boarding_cooldown <= 0 && !matches!(species, Species::Warden | Species::Wither)
}

/// `Entity.positionRider` on an `AbstractMinecart`: the seat offset
/// (`cart_seat_offset`) above the cart.
pub fn cart_seat(cart: Vec3, seat_offset: f64) -> Vec3 {
    Vec3::new(cart.x, cart.y + seat_offset, cart.z)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    NorthSouth,
    EastWest,
    AscendingEast,
    AscendingWest,
    AscendingNorth,
    AscendingSouth,
    SouthEast,
    SouthWest,
    NorthWest,
    NorthEast,
}
impl Shape {
    pub fn parse(s: &str) -> Self {
        match s {
            "east_west" => Self::EastWest,
            "ascending_east" => Self::AscendingEast,
            "ascending_west" => Self::AscendingWest,
            "ascending_north" => Self::AscendingNorth,
            "ascending_south" => Self::AscendingSouth,
            "south_east" => Self::SouthEast,
            "south_west" => Self::SouthWest,
            "north_west" => Self::NorthWest,
            "north_east" => Self::NorthEast,
            _ => Self::NorthSouth,
        }
    }
    pub fn is_slope(self) -> bool {
        matches!(
            self,
            Self::AscendingEast | Self::AscendingWest | Self::AscendingNorth | Self::AscendingSouth
        )
    }
    /// `AbstractMinecart.EXITS`.
    pub fn exits(self) -> ([i32; 3], [i32; 3]) {
        let (w, e, n, s) = ([-1, 0, 0], [1, 0, 0], [0, 0, -1], [0, 0, 1]);
        let below = |v: [i32; 3]| [v[0], -1, v[2]];
        match self {
            Self::NorthSouth => (n, s),
            Self::EastWest => (w, e),
            Self::AscendingEast => (below(w), e),
            Self::AscendingWest => (w, below(e)),
            Self::AscendingNorth => (n, below(s)),
            Self::AscendingSouth => (below(n), s),
            Self::SouthEast => (s, e),
            Self::SouthWest => (s, w),
            Self::NorthWest => (n, w),
            Self::NorthEast => (n, e),
        }
    }
}
pub struct Rail {
    pub name: Arc<str>,
    pub shape: Shape,
    pub powered: bool,
}
impl Minecart {
    /// `Minecart.tick`: stationary rotation does not turn the passenger.
    pub fn accumulate_rotation(&mut self) {
        self.passenger_yaw_delta = 0.0;
        if self.kind == CartKind::Rideable && self.position.subtract(self.previous).length() > 0.01
        {
            self.rotation_offset +=
                ((f64::from(self.yaw) - f64::from(self.previous_yaw)) % 360.0) as f32;
            self.rotation_offset %= 360.0;
        }
    }
    /// `Minecart.positionRider`: double rotLerp, then round back to float.
    /// Only yaw follows; the player's pitch and manual look remain their own.
    pub fn rotate_passenger(&mut self, yaw: &mut f32) {
        let mut delta =
            (f64::from(self.rotation_offset) - f64::from(self.player_rotation_offset)) % 360.0;
        if delta >= 180.0 {
            delta -= 360.0;
        }
        if delta < -180.0 {
            delta += 360.0;
        }
        let next = (f64::from(self.player_rotation_offset) + 0.5 * delta) as f32;
        let previous = *yaw;
        *yaw -= next - self.player_rotation_offset;
        self.passenger_yaw_delta = *yaw - previous;
        self.player_rotation_offset = next;
    }
    /// `Entity.onGround`.
    pub fn on_ground(&self) -> bool {
        self.on_ground
    }
    pub fn new(id: u32, kind: CartKind, position: Vec3) -> Self {
        Self {
            join: Default::default(),
            attachment_lifetime: Default::default(),
            id,
            kind,
            position,
            previous: position,
            velocity: Vec3::ZERO,
            yaw: 0.0,
            previous_yaw: 0.0,
            pitch: 0.0,
            previous_pitch: 0.0,
            rotation_offset: 0.0,
            player_rotation_offset: 0.0,
            passenger_yaw_delta: 0.0,
            lerp_steps: Vec::new(),
            flipped: false,
            on_rails: false,
            on_ground: false,
            horizontal_collision: false,
            fall_distance: 0.0,
            fall_blast: None,
            stuck: Vec3::ZERO,
            hurt_time: 0,
            hurt_dir: 1,
            damage: 0.0,
            no_gravity: false,
            ticks_frozen: 0,
            cactus_hits: 0,
            items: vec![None; kind.slots()],
            fuel: 0,
            push: Vec3::ZERO,
            fuse: -1,
            explosion_power: DEFAULT_EXPLOSION_POWER,
            explosion_speed_factor: DEFAULT_EXPLOSION_SPEED_FACTOR,
            enabled: true,
            loot: None,
            first_tick: true,
            in_water: false,
            command: horizon_client::command_block::CommandBlock::default(),
            tick_count: 0,
            last_activated: 0,
            command_due: false,
            // A fresh `BaseSpawner`, as `MinecartSpawner`'s constructor makes.
            spawner: (kind == CartKind::Spawner).then(Default::default),
            synced_command: String::new(),
            synced_output: String::new(),
            custom_display: None,
            custom_offset: None,
            random: EntityRandom::own(horizon_client::random_source::entity()),
            near_carts: None,
        }
    }
    pub fn bounds(&self) -> Aabb {
        let (w, p) = (WIDTH / 2.0, self.position);
        Aabb::new(
            Vec3::new(p.x - w, p.y, p.z - w),
            Vec3::new(p.x + w, p.y + HEIGHT, p.z + w),
        )
    }
    pub fn horizontal(&self) -> f64 {
        self.velocity.x.hypot(self.velocity.z)
    }
    /// `AbstractContainerMenu.getRedstoneSignalFromContainer` of `slots`.
    pub fn container_signal(slots: &[Option<ItemStack>], items: &Items) -> i32 {
        let mut total = 0.0f32;
        for s in slots.iter().flatten() {
            let max = items.get(&s.item).map_or(64, |i| i.max_stack).min(64);
            total += s.count as f32 / max as f32;
        }
        total /= slots.len().max(1) as f32;
        // Mth.lerpDiscrete(total, 0, 15)
        (total * 14.0).floor() as i32 + i32::from(total > 0.0)
    }
}
/// `AbstractMinecartContainer.applyNaturalSlowdown`'s keep factor, in the
/// float arithmetic vanilla uses: 0.98 plus a thousandth per empty step of the
/// comparator signal (none while a loot table is still unrolled), times 0.95
/// in water.
pub fn container_keep(signal: i32, loot_pending: bool, in_water: bool) -> f64 {
    let mut keep = 0.98_f32;
    if !loot_pending {
        keep += (15 - signal) as f32 * 0.001_f32;
    }
    if in_water {
        keep *= 0.95_f32;
    }
    f64::from(keep)
}
pub struct CartModels {
    pub models: HashMap<CartKind, Part>,
}
impl CartModels {
    pub fn load(document: Option<&horizon_client::mobs::EntityModelDocument>) -> Self {
        let kinds = [
            CartKind::Rideable,
            CartKind::Chest,
            CartKind::Hopper,
            CartKind::Furnace,
            CartKind::Tnt,
            CartKind::CommandBlock,
            CartKind::Spawner,
        ];
        let models = kinds
            .into_iter()
            .filter_map(|k| document?.model(&format!("{}#main", k.id())).map(|m| (k, m)))
            .collect();
        Self { models }
    }
}
/// Saving and loading.
impl Minecart {
    pub fn to_nbt(
        &self,
        uuid: [i32; 4],
        raw: Option<&horizon_core::nbt::Tag>,
    ) -> horizon_core::nbt::Tag {
        use horizon_core::nbt::{JavaString, Tag};
        let mut tag = horizon_client::entity_nbt::vehicle_to_nbt(
            self.kind.id(),
            self.position,
            self.velocity,
            [self.yaw, self.pitch],
            (self.on_ground, self.fall_distance),
            uuid,
            raw,
        );
        horizon_client::entity_nbt::write_saved_flags(&mut tag, self.on_ground, self.no_gravity);
        if let Tag::Compound(map) = &mut tag {
            // ContainerEntity.addChestVehicleSaveData belongs only to the
            // container subclasses, never to a plain or specialized cart.
            if self.kind.slots() == 0 {
                map.remove(&JavaString::from("Items"));
            }
            if self.kind == CartKind::CommandBlock {
                for key in ["LastOutput", "LastExecution"] {
                    map.remove(&JavaString::from(key));
                }
            }
            map.remove(&JavaString::from("DisplayOffset"));
            map.remove(&JavaString::from("DisplayState"));
            map.remove(&JavaString::from("explosion_power"));
            map.remove(&JavaString::from("explosion_speed_factor"));
            put_ticks_frozen(map, self.ticks_frozen);
            let loaded_fire = map
                .get(&JavaString::from("Fire"))
                .and_then(horizon_client::nbt_path::int_value)
                .unwrap_or(0) as i16;
            let mut put = |k: &str, v: Tag| {
                map.insert(JavaString::from(k), v);
            };
            put("FlippedRotation", Tag::Byte(i8::from(self.flipped)));
            // AbstractMinecart saves firstTick as HasTicked. A cart that is not
            // on fire never burns: Entity.move sets -getFireImmuneTicks(), 0 for
            // all but players. One loaded on fire stays so, with no baseTick to
            // count its fire ticks down.
            put("HasTicked", Tag::Byte(i8::from(self.first_tick)));
            put("Fire", Tag::Short(loaded_fire.max(0)));
            // addAdditionalSaveData: the offset only off its default, the state only when set.
            if let Some(offset) = self.custom_offset
                && f64::from(offset) != default_display_offset(self.kind)
            {
                put("DisplayOffset", Tag::Int(offset));
            }
            if let Some(state) = &self.custom_display {
                put("DisplayState", state.clone());
            }
            match self.kind {
                CartKind::Chest | CartKind::Hopper => {
                    put("Items", horizon_client::save::encode_items(&self.items));
                    if self.kind == CartKind::Hopper {
                        put("Enabled", Tag::Byte(i8::from(self.enabled)));
                    }
                }
                CartKind::Furnace => {
                    put("PushX", Tag::Double(self.push.x.to_bits()));
                    put("PushZ", Tag::Double(self.push.z.to_bits()));
                    put("Fuel", Tag::Short(self.fuel as i16));
                }
                // MinecartTNT.addAdditionalSaveData: the powers only off their defaults.
                CartKind::Tnt => {
                    put("fuse", Tag::Int(self.fuse));
                    for (key, value, default) in [
                        (
                            "explosion_power",
                            self.explosion_power,
                            DEFAULT_EXPLOSION_POWER,
                        ),
                        (
                            "explosion_speed_factor",
                            self.explosion_speed_factor,
                            DEFAULT_EXPLOSION_SPEED_FACTOR,
                        ),
                    ] {
                        if value != default {
                            put(key, Tag::Float(value.to_bits()));
                        }
                    }
                }
                // BaseCommandBlock.save (its CustomName shares the entity's key).
                CartKind::CommandBlock => {
                    for (key, value) in self.command.save() {
                        if !matches!(key, "CustomName" | "powered" | "conditionMet" | "auto") {
                            put(key, value);
                        }
                    }
                }
                // BaseSpawner.save: the delays, ranges, SpawnData and SpawnPotentials.
                CartKind::Spawner => {
                    if let Some(spawner) = &self.spawner {
                        for (key, value) in
                            horizon_client::mobs::spawner::SpawnerExt::save(&**spawner)
                        {
                            put(key, value);
                        }
                    }
                }
                CartKind::Rideable => {}
            }
            map.remove(&JavaString::from("LootTable"));
            map.remove(&JavaString::from("LootTableSeed"));
            if let Some(pending) = &self.loot {
                let items = map.get(&"Items".into()).cloned();
                pending.write_tags(map);
                if self.items.iter().any(Option::is_some) {
                    map.extend(items.map(|items| ("Items".into(), items)));
                }
            }
        }
        tag
    }
    pub fn from_nbt(tag: &horizon_core::nbt::Tag, id: u32, kind: CartKind) -> Option<Self> {
        let (position, _, _) = horizon_client::entity_nbt::boat_from_nbt(tag)?;
        let mut c = Self::new(id, kind, position);
        c.load(tag)?;
        Some(c)
    }
    /// `Entity.load` on this cart: the saved fields are read over the live
    /// ones, the rest (riders, lerp, hurt state) stays; its kind never changes.
    /// `None`, with nothing changed, when the compound has no position.
    pub fn load(&mut self, tag: &horizon_core::nbt::Tag) -> Option<()> {
        use horizon_core::nbt::Tag;
        let (position, velocity, yaw) = horizon_client::entity_nbt::boat_from_nbt(tag)?;
        let kind = self.kind;
        let c = self;
        c.position = position;
        c.previous = position;
        c.velocity = velocity;
        c.yaw = yaw;
        c.previous_yaw = yaw;
        c.no_gravity = horizon_client::entity_nbt::flag(tag, "NoGravity");
        c.on_ground = horizon_client::entity_nbt::flag(tag, "OnGround");
        c.pitch = horizon_client::entity_nbt::rotation(tag)[1];
        c.previous_pitch = c.pitch;
        // ValueInput's numeric getters take any NumericTag: getIntOr is its
        // intValue, getShortOr the low 16 bits of it, getBooleanOr
        // `byteValue() != 0` (the low 8), getDoubleOr its doubleValue.
        let int = |k: &str| tag.get(k).and_then(horizon_client::nbt_path::int_value);
        let flag = |k: &str, default: bool| int(k).map_or(default, |v| v as i8 != 0);
        let double = |k: &str| match tag.get(k) {
            Some(Tag::Byte(v)) => f64::from(*v),
            Some(Tag::Short(v)) => f64::from(*v),
            Some(Tag::Int(v)) => f64::from(*v),
            Some(Tag::Long(v)) => *v as f64,
            Some(Tag::Float(v)) => f64::from(f32::from_bits(*v)),
            Some(Tag::Double(v)) => f64::from_bits(*v),
            _ => 0.0,
        };
        // Entity.load: OnGround and fall_distance, as saved.
        (c.on_ground, c.fall_distance) = horizon_client::entity_nbt::ground_from_nbt(tag);
        c.ticks_frozen = saved_ticks_frozen(tag);
        c.flipped = flag("FlippedRotation", false);
        c.first_tick = flag("HasTicked", false);
        if kind.slots() > 0 {
            c.loot = horizon_client::save::PendingContainerLoot::from_tag(tag);
            c.items = if c.loot.is_some() {
                vec![None; kind.slots()]
            } else {
                horizon_client::save::decode_items(tag.get("Items"), kind.slots())
            };
        }
        c.enabled = flag("Enabled", true);
        c.fuel = int("Fuel").map_or(0, |v| i32::from(v as i16));
        // readAdditionalSaveData: the saved push has no y.
        c.push = Vec3::new(double("PushX"), 0.0, double("PushZ"));
        c.fuse = int("fuse").unwrap_or(-1);
        if kind == CartKind::Tnt {
            // MinecartTNT.readAdditionalSaveData: getFloatOr, kept to 0..128.
            let float = |k: &str, default: f32| match tag.get(k) {
                Some(Tag::Float(v)) => f32::from_bits(*v),
                Some(Tag::Double(v)) => f64::from_bits(*v) as f32,
                Some(Tag::Long(v)) => *v as f32,
                Some(other) => {
                    horizon_client::nbt_path::int_value(other).map_or(default, |v| v as f32)
                }
                None => default,
            };
            c.explosion_power = float("explosion_power", DEFAULT_EXPLOSION_POWER).clamp(0.0, 128.0);
            c.explosion_speed_factor =
                float("explosion_speed_factor", DEFAULT_EXPLOSION_SPEED_FACTOR).clamp(0.0, 128.0);
        }
        // readAdditionalSaveData: DisplayState, DisplayOffset (default offset when absent).
        c.custom_display = tag.get("DisplayState").cloned();
        c.custom_offset = int("DisplayOffset");
        if kind == CartKind::CommandBlock {
            c.command = horizon_client::command_block::CommandBlock::load(tag);
            // MinecartCommandBlock.readAdditionalSaveData sets the synched command.
            // A changed value invokes onSyncedDataUpdated -> setCommand, resetting successes.
            if c.synced_command != c.command.command {
                c.synced_command.clone_from(&c.command.command);
                c.command.set_command(&c.synced_command);
            }
            c.synced_output = c.command.last_output_string();
        }
        // MinecartSpawner.readAdditionalSaveData: BaseSpawner.load.
        if kind == CartKind::Spawner {
            c.spawner = Some(Box::new(horizon_client::mobs::spawner::SpawnerExt::load(
                tag,
            )));
        }
        Some(())
    }
}

/// `MinecartTNT` default explosion powers.
pub const DEFAULT_EXPLOSION_POWER: f32 = 4.0;
pub const DEFAULT_EXPLOSION_SPEED_FACTOR: f32 = 1.0;

/// `VehicleEntity.hurtServer`'s destroy test: a creative player, damage past
/// 40, or (`MinecartTNT.shouldSourceDestroy`) fire or an explosion hitting a
/// TNT cart.
pub fn cart_is_destroyed(creative: bool, damage: f32, ignites_tnt: bool) -> bool {
    creative || damage > 40.0 || ignites_tnt
}

/// `MinecartTNT.tick`'s collision detonation: a cart still moving at a tenth
/// of a block a tick or more when a block stops it blows up.
pub fn tnt_ignites_by_collision(horizontal_collision: bool, speed_sqr: f64) -> bool {
    horizontal_collision && speed_sqr >= 0.009_999_999_776_482_582
}

/// NewMinecartBehavior's `(float) (Math.atan2(y, x) * 180.0 / Math.PI)` before
/// the cast: HotSpot's atan2, multiplied then divided in Java's order.
pub fn java_degrees(y: f64, x: f64) -> f64 {
    horizon_core::jmath::atan2(y, x) * 180.0 / std::f64::consts::PI
}

/// `MinecartFurnace.addFuel`: Java int arithmetic, so a fuel a mod set near
/// `i32::MAX` wraps negative and takes more.
pub fn furnace_fuel_after(fuel: i32) -> Option<i32> {
    let next = fuel.wrapping_add(3600);
    (next <= 32000).then_some(next)
}

/// `MinecartFurnace.getMaxSpeed`: half the behavior's speed, three quarters
/// in water.
pub fn furnace_speed(c: &Minecart, base: f64) -> f64 {
    if c.kind == CartKind::Furnace {
        base * if c.in_water { 0.75 } else { 0.5 }
    } else {
        base
    }
}
horizon_client::impl_joined!(Minecart);
horizon_mod_runtime::impl_attachment_lifetime!(Minecart);
impl horizon_client::entity_rules::EntityRules for Minecart {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// `AbstractMinecart.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        if self.in_water { 0.005 } else { 0.04 }
    }
}
