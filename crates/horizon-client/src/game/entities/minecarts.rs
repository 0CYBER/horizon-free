//! Minecarts (`AbstractMinecart` with `OldMinecartBehavior`): following
//! rails and slopes, powered and activator rails, coming off the track,
//! riding with the rider's push, pushing by players and other carts,
//! `MinecartItem` placement, `VehicleEntity` breaking, the chest, hopper,
//! furnace and TNT variants, saving, and `AbstractMinecartRenderer`'s old
//! rail-following render with the display block.
#[cfg(test)]
#[path = "minecarts/lifetime_tests.rs"]
mod lifetime_tests;
#[path = "minecarts/save_lending.rs"]
mod save_lending;
#[path = "minecarts/section_steps.rs"]
mod section_steps;

use super::{Game, Hit, InteractTarget, Keys, entity::Entity};
use horizon_client::actions::Action;
use horizon_client::entity_rules::EntityRules;
use horizon_client::fast_hash::FxHashSet;
use horizon_client::join_seq::{JoinList, ListStamp};
use horizon_client::mobs::{Mob, seen_grid::SeenGrid};
use horizon_client::{
    blocks::Fluid,
    containers::BlockEntity,
    items::{ItemStack, Items},
    mesher::Vertex,
    mobs::{Part, Species, Who, anim::Poses, riders},
    particles::{ParticleOptions, ParticleType},
    random_source,
    scoreboard::pushable_by,
    sound_engine::{BLOCKS, CartSound, PlayResult, SoundInstance, Sounds},
};
use horizon_core::{
    geometry::{Aabb, Vec3, trig},
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
use horizon_world::registry::BlockStateId;
use std::{collections::HashMap, sync::Arc};
#[path = "minecarts/cells.rs"]
mod cells;
#[path = "minecarts/hit.rs"]
mod hit;
#[path = "minecarts/seat.rs"]
mod seat;
pub(super) use hit::CartHit;
use seat::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum CartKind {
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
    pub(super) fn slots(self) -> usize {
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
#[derive(Clone, Debug)]
pub(super) struct Minecart {
    pub(super) attachment_lifetime: horizon_mod_runtime::attachments::AttachmentLifetime,
    /// When it joined its list (`horizon_client::join_seq`); set by the list.
    pub(super) join: horizon_client::join_seq::JoinSeq,
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
    pub(super) on_rails: bool,
    pub(super) on_ground: bool,
    pub(super) horizontal_collision: bool,
    pub(super) fall_distance: f64,
    /// The squared speed `MinecartTNT.causeFallDamage` blows up with, set by
    /// a landing of three blocks or more and taken by the cart's next tick
    /// step (vanilla explodes inside the move itself).
    pub(super) fall_blast: Option<f64>,
    /// `Entity.stuckSpeedMultiplier`: set by a cobweb the cart's box is in,
    /// scaled into its next move.
    pub(super) stuck: Vec3,
    pub hurt_time: i32,
    pub hurt_dir: i32,
    pub damage: f32,
    /// `Entity.noGravity` (`NoGravity` in its save).
    pub no_gravity: bool,
    /// `Entity.ticksFrozen` (`TicksFrozen` in its save): powder snow builds
    /// it up (`InsideBlockEffectType.FREEZE`), a cart never melts it.
    pub ticks_frozen: i32,
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
    pub random: LegacyRandom,
}
/// `EntityType.MINECART`'s `sized(0.98F, 0.7F)`: float dimensions, widened.
pub(super) const WIDTH: f64 = 0.98_f32 as f64;
pub(super) const HEIGHT: f64 = 0.7_f32 as f64;
/// `EntityEvent.TNT_PRIME`. `MinecartTNT.primeFuse` broadcasts it;
/// `MinecartTNT.handleEntityEvent` answers by calling `primeFuse`.
pub(super) const TNT_PRIME_EVENT: i8 = 70;
/// `AbstractMinecart.getDefaultDisplayOffset` (6), in sixteenths of a block, as
/// `MinecartChest` (8) and `MinecartHopper` (1) override it.
pub(super) fn default_display_offset(kind: CartKind) -> f64 {
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
pub(super) fn display_offset(c: &Minecart) -> f64 {
    c.custom_offset
        .map_or_else(|| default_display_offset(c.kind), f64::from)
}

/// World-space envelope of the cart model and its optional displayed block,
/// based on the same root position and display offset used by the renderer.
fn cart_render_bounds(position: Vec3, offset: f64, has_display: bool) -> Aabb {
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
fn shown_unless_air(
    state: Option<BlockStateId>,
    is_air: impl Fn(BlockStateId) -> bool,
) -> Option<BlockStateId> {
    state.filter(|&s| !is_air(s))
}
/// `readAdditionalSaveData`'s `input.read("DisplayState", CODEC)` is empty
/// when the state does not decode: the cart forgets it (and no longer saves it).
pub(super) fn forget_unreadable_display(
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
pub(super) struct CartIndex {
    carts: Option<(SeenGrid, Vec<u32>)>,
    /// The grid, and the carts a mob rides.
    mobs: Option<(ListStamp, SeenGrid, FxHashSet<u32>)>,
    near: Vec<u32>,
}
impl CartIndex {
    fn usable() -> bool {
        mod_api::__private::mods().is_empty()
    }
    /// The cart in slot `i` has these bounds now.
    fn moved(&mut self, i: usize, bounds: Aabb) {
        if let Some((grid, ids)) = &mut self.carts
            && i < ids.len()
        {
            grid.update(i, bounds);
        }
    }
    fn forget_carts(&mut self) {
        self.carts = None;
    }
    /// The slots of the carts whose box may meet `area`, for the cart in slot
    /// `i` (just moved); false: scan them all.
    fn carts_near(&mut self, carts: &[Minecart], i: usize, area: Aabb, out: &mut Vec<u32>) -> bool {
        if !Self::usable() {
            return false;
        }
        let fresh = |ids: &[u32]| ids.len() == carts.len() && ids[i] == carts[i].id;
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
    /// The slots of the mobs whose box may meet `area` into `out`, and
    /// whether a mob rides cart `cart`; None: scan them all.
    fn mobs_near(
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
pub(in crate::game) fn cart_at(carts: &[Minecart], id: u32, hint: usize) -> Option<usize> {
    if carts.get(hint).is_some_and(|c| c.id == id) {
        return Some(hint);
    }
    carts.iter().position(|c| c.id == id)
}
/// `AbstractMinecart.push(entity)` for a non-cart `entity` at `feet`:
/// the push the cart loses (the entity gets a quarter of it back).
pub(super) fn cart_push_from(feet: Vec3, cart: Vec3) -> Option<(f64, f64)> {
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
fn push_along(push: Vec3, movement: Vec3) -> Vec3 {
    if push.horizontal_distance_squared() > 1.0e-4 && movement.horizontal_distance_squared() > 0.001
    {
        push.projected_on(movement).normalize().scale(push.length())
    } else {
        push
    }
}
/// The minecart's passenger attachment minus Player.DEFAULT_VEHICLE_ATTACHMENT.
pub(super) const SEAT: f64 = PASSENGER_Y - 0.6;
/// The placeholder container key under which an open cart's items live.
pub(super) const CART_KEY_Y: i32 = horizon_client::lan::menu::VEHICLE_KEY_Y;

/// `OldMinecartBehavior.pushAndPickupEntities`: living entities the cart
/// offers `startRiding` instead of `Entity.push`. Players click to board;
/// iron golems always push; a cart that already has a rider is `isVehicle`.
fn pickup_instead_of_push(species: Species, is_passenger: bool, cart_is_vehicle: bool) -> bool {
    !cart_is_vehicle && !is_passenger && species != Species::IronGolem
}

/// `EntitySelector.pushableBy` asks `isPushable`: `Shulker` takes no push
/// (`Mob::pushable` leaves it out) but still counts, so a cart offers it
/// `startRiding` like any other living entity.
fn offered_pickup(pushable: bool, species: Species, alive: bool) -> bool {
    pushable || species == Species::Shulker && alive
}

/// `Shulker.push(Entity)` is empty, so a shulker that is not picked up (it
/// already rides something) neither moves nor slows the cart.
fn takes_push_from_cart(species: Species) -> bool {
    species != Species::Shulker
}

/// `Entity.canRide` for a cart: the boarding cooldown, and a warden or the
/// wither never rides (`canRide` false).
fn can_ride_cart(species: Species, boarding_cooldown: i32) -> bool {
    boarding_cooldown <= 0 && !matches!(species, Species::Warden | Species::Wither)
}

/// `Entity.positionRider` on an `AbstractMinecart`: the seat offset
/// (`cart_seat_offset`) above the cart.
fn cart_seat(cart: Vec3, seat_offset: f64) -> Vec3 {
    Vec3::new(cart.x, cart.y + seat_offset, cart.z)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Shape {
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
    pub(super) fn parse(s: &str) -> Self {
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
    pub(super) fn is_slope(self) -> bool {
        matches!(
            self,
            Self::AscendingEast | Self::AscendingWest | Self::AscendingNorth | Self::AscendingSouth
        )
    }
    /// `AbstractMinecart.EXITS`.
    fn exits(self) -> ([i32; 3], [i32; 3]) {
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
pub(super) struct Rail {
    pub(super) name: Arc<str>,
    pub(super) shape: Shape,
    pub(super) powered: bool,
}
impl Minecart {
    /// `Minecart.tick`: stationary rotation does not turn the passenger.
    pub(super) fn accumulate_rotation(&mut self) {
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
    pub(super) fn rotate_passenger(&mut self, yaw: &mut f32) {
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
    pub(super) fn on_ground(&self) -> bool {
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
            random: random_source::entity(),
        }
    }
    pub fn bounds(&self) -> Aabb {
        let (w, p) = (WIDTH / 2.0, self.position);
        Aabb::new(
            Vec3::new(p.x - w, p.y, p.z - w),
            Vec3::new(p.x + w, p.y + HEIGHT, p.z + w),
        )
    }
    fn horizontal(&self) -> f64 {
        self.velocity.x.hypot(self.velocity.z)
    }
    /// `AbstractContainerMenu.getRedstoneSignalFromContainer` of `slots`.
    pub(super) fn container_signal(slots: &[Option<ItemStack>], items: &Items) -> i32 {
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
fn container_keep(signal: i32, loot_pending: bool, in_water: bool) -> f64 {
    let mut keep = 0.98_f32;
    if !loot_pending {
        keep += (15 - signal) as f32 * 0.001_f32;
    }
    if in_water {
        keep *= 0.95_f32;
    }
    f64::from(keep)
}
pub(super) struct CartModels {
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
impl Game<'_> {
    /// `getCurrentBlockPosOrRailBelow`.
    fn rail_block(&self, p: Vec3) -> [i32; 3] {
        let (x, y, z) = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        let below = if self.minecart_improvements() {
            (p.y - 0.1 - f64::from(1.0e-5_f32)).floor() as i32
        } else {
            y - 1
        };
        if self.rail(None, x, below, z).is_some() {
            [x, below, z]
        } else {
            [x, y, z]
        }
    }
    /// `AbstractMinecart.isRedstoneConductor`: the block state's own flag
    /// (soul sand conducts without a full collision shape).
    fn conductor(&self, x: i32, y: i32, z: i32) -> bool {
        self.res.behaviours().facts(self.world.block(x, y, z)).flags
            & horizon_world::level::data::state_flags::CONDUCTOR
            != 0
    }
    /// `OldMinecartBehavior.getPos`: the point on the rail under (x, y, z), on the track the
    /// rail lays for `cart`.
    fn rail_pos(&self, cart: Option<u32>, x: f64, y: f64, z: f64) -> Option<[f64; 3]> {
        let [xt, yt, zt] = self.rail_block(Vec3::new(x, y, z));
        let rail = self.rail(cart, xt, yt, zt)?;
        let (e0, e1) = rail.shape.exits();
        let (fx, fy, fz) = (f64::from(xt), f64::from(yt), f64::from(zt));
        let x0 = fx + 0.5 + f64::from(e0[0]) * 0.5;
        let y0 = fy + 0.0625 + f64::from(e0[1]) * 0.5;
        let z0 = fz + 0.5 + f64::from(e0[2]) * 0.5;
        let x1 = fx + 0.5 + f64::from(e1[0]) * 0.5;
        let y1 = fy + 0.0625 + f64::from(e1[1]) * 0.5;
        let z1 = fz + 0.5 + f64::from(e1[2]) * 0.5;
        let (xd, yd, zd) = (x1 - x0, (y1 - y0) * 2.0, z1 - z0);
        let progress = if xd == 0.0 {
            z - fz
        } else if zd == 0.0 {
            x - fx
        } else {
            ((x - x0) * xd + (z - z0) * zd) * 2.0
        };
        let mut out = [x0 + xd * progress, y0 + yd * progress, z0 + zd * progress];
        if yd < 0.0 {
            out[1] += 1.0;
        } else if yd > 0.0 {
            out[1] += 0.5;
        }
        Some(out)
    }
    /// `OldMinecartBehavior.getPosOffs`.
    fn rail_pos_offs(
        &self,
        cart: Option<u32>,
        mut x: f64,
        y: f64,
        mut z: f64,
        offs: f64,
    ) -> Option<[f64; 3]> {
        let [xt, yt, zt] = self.rail_block(Vec3::new(x, y, z));
        let rail = self.rail(cart, xt, yt, zt)?;
        let mut y = f64::from(yt) + if rail.shape.is_slope() { 1.0 } else { 0.0 };
        let (e0, e1) = rail.shape.exits();
        let (mut xd, mut zd) = (f64::from(e1[0] - e0[0]), f64::from(e1[2] - e0[2]));
        let dd = xd.hypot(zd);
        xd /= dd;
        zd /= dd;
        x += xd * offs;
        z += zd * offs;
        let at = |e: [i32; 3]| x.floor() as i32 - xt == e[0] && z.floor() as i32 - zt == e[2];
        if e0[1] != 0 && at(e0) {
            y += f64::from(e0[1]);
        } else if e1[1] != 0 && at(e1) {
            y += f64::from(e1[1]);
        }
        self.rail_pos(cart, x, y, z)
    }
    /// `AbstractMinecart.canCollideWith`, for `Entity.collide`'s `getEntityCollisions`:
    /// `AbstractBoat.canVehicleCollide` takes any other cart, boat, shulker or pushable mob and
    /// player, except one riding this cart. A cart that runs into one has a horizontal
    /// collision, which sets a TNT minecart off.
    pub(super) fn cart_entity_collisions(&self, c: &Minecart) -> Vec<Aabb> {
        let near = c.bounds().inflate(Vec3::new(2.0, 2.0, 2.0));
        // Every moving cart asks each tick, and the box turns nearly every entity of
        // the level away: it is tested first, and only the mobs near it are read.
        let close = |b: &Aabb| b.intersects(near);
        let mut boxes: Vec<Aabb> = self
            .carts
            .iter()
            .filter(|o| o.id != c.id)
            .map(Minecart::bounds)
            .filter(close)
            .chain(
                self.boats
                    .iter()
                    .map(super::boats::Boat::bounds)
                    .filter(close),
            )
            .collect();
        self.mobs_meeting(near, |_, m, b| {
            let counts = m.pushable() || m.species() == Species::Shulker && m.alive();
            if !m.removed && m.extra.mount != Some(c.id) && counts {
                boxes.push(b);
            }
        });
        if self.riding != Some(c.id)
            && let Some(p) = self.player.as_ref()
            && !self.spectator()
            && close(&p.bounds())
        {
            boxes.push(p.bounds());
        }
        if let Some(host) = self.lan.as_ref() {
            boxes.extend(
                host.hub
                    .guests
                    .iter()
                    .filter(|g| {
                        host.hub.here(g)
                            && g.loaded
                            && !g.dead
                            && !g.spectator()
                            && g.vehicle != Some(c.id)
                    })
                    .map(|g| g.bounds())
                    .filter(close),
            );
        }
        boxes
    }
    fn max_speed(&self, c: &Minecart) -> f64 {
        let base = if c.in_water { 0.2 } else { 0.4 };
        furnace_speed(c, base)
    }
    /// `AbstractMinecart.getMaxSpeed`: the behavior's, which a furnace
    /// minecart cuts (`MinecartFurnace.getMaxSpeed`).
    pub(super) fn cart_max_speed(&self, c: &Minecart) -> f64 {
        if self.minecart_improvements() {
            furnace_speed(c, self.max_speed_new(c))
        } else {
            self.max_speed(c)
        }
    }
    /// `applyNaturalSlowdown` (with `MinecartFurnace`'s push); `factor` is
    /// `getSlowdownFactor`: 0.997 riding, the empty cart's own by behavior.
    fn natural_slowdown(&self, c: &mut Minecart, ridden: bool, factor: f64) -> Vec3 {
        let mut v = c.velocity;
        let water = c.in_water;
        if c.kind == CartKind::Furnace {
            if c.push.length_squared() > 1.0e-7 {
                c.push = push_along(c.push, v);
                v = Vec3::new(v.x * 0.8 + c.push.x, 0.0, v.z * 0.8 + c.push.z);
                if water {
                    v = Vec3::new(v.x * 0.1, 0.0, v.z * 0.1);
                }
            } else {
                v = Vec3::new(v.x * 0.98, 0.0, v.z * 0.98);
            }
        }
        if matches!(c.kind, CartKind::Chest | CartKind::Hopper) {
            // `AbstractMinecartContainer.applyNaturalSlowdown` replaces the
            // base one whole: the keep factor leans on how empty it is, and
            // riders and the behavior's own factor do not enter.
            let signal = Minecart::container_signal(&c.items, self.items());
            let keep = container_keep(signal, c.loot.is_some(), water);
            return Vec3::new(v.x * keep, 0.0, v.z * keep);
        }
        let f = if ridden { 0.997 } else { factor };
        let mut out = Vec3::new(v.x * f, 0.0, v.z * f);
        if water {
            // scale(0.95F): the float widened.
            let drag = f64::from(0.95_f32);
            out = Vec3::new(out.x * drag, 0.0, out.z * drag);
        }
        out
    }
    /// `Entity.isVehicle` for a cart: the local player, a LAN guest or a mob
    /// sits in it. `getSlowdownFactor` (0.997) and the 0.75 speed scale key on
    /// this, not on who rides.
    fn cart_is_vehicle(&self, id: u32) -> bool {
        self.riding == Some(id) || self.ridden_by_guest(id) || self.mob_rides(id)
    }
    /// `OldMinecartBehavior.moveAlongTrack`.
    fn move_along_track(
        &mut self,
        c: &mut Minecart,
        pos: [i32; 3],
        rail: &Rail,
        intent: [f64; 2],
        slot: usize,
    ) {
        c.fall_distance = 0.0;
        let ridden = self.cart_is_vehicle(c.id);
        let (x, z) = (c.position.x, c.position.z);
        let old = self.rail_pos(Some(c.id), x, c.position.y, z);
        let mut y = f64::from(pos[1]);
        let (mut power, mut halt) = (false, false);
        if &*rail.name == "minecraft:powered_rail" {
            power = rail.powered;
            halt = !power;
        }
        let slide = if c.in_water {
            0.0078125 * 0.2
        } else {
            0.0078125
        };
        match rail.shape {
            Shape::AscendingEast => c.velocity.x -= slide,
            Shape::AscendingWest => c.velocity.x += slide,
            Shape::AscendingNorth => c.velocity.z += slide,
            Shape::AscendingSouth => c.velocity.z -= slide,
            _ => {}
        }
        if rail.shape.is_slope() {
            y += 1.0;
        }
        let (e0, e1) = rail.shape.exits();
        let (mut xd, mut zd) = (f64::from(e1[0] - e0[0]), f64::from(e1[2] - e0[2]));
        let length = xd.hypot(zd);
        if c.velocity.x * xd + c.velocity.z * zd < 0.0 {
            xd = -xd;
            zd = -zd;
        }
        let pow = c.horizontal().min(2.0);
        c.velocity = Vec3::new(pow * xd / length, c.velocity.y, pow * zd / length);
        if ridden
            && (intent[0] != 0.0 || intent[1] != 0.0)
            && c.velocity.x.powi(2) + c.velocity.z.powi(2) < 0.01
        {
            c.velocity.x += intent[0] * 0.001;
            c.velocity.z += intent[1] * 0.001;
            halt = false;
        }
        if halt {
            if c.horizontal() < 0.03 {
                c.velocity = Vec3::ZERO;
            } else {
                c.velocity = Vec3::new(c.velocity.x * 0.5, 0.0, c.velocity.z * 0.5);
            }
        }
        let (fx, fz) = (f64::from(pos[0]), f64::from(pos[2]));
        let x0 = fx + 0.5 + f64::from(e0[0]) * 0.5;
        let z0 = fz + 0.5 + f64::from(e0[2]) * 0.5;
        let x1 = fx + 0.5 + f64::from(e1[0]) * 0.5;
        let z1 = fz + 0.5 + f64::from(e1[2]) * 0.5;
        let (xd, zd) = (x1 - x0, z1 - z0);
        let progress = if xd == 0.0 {
            z - fz
        } else if zd == 0.0 {
            x - fx
        } else {
            ((x - x0) * xd + (z - z0) * zd) * 2.0
        };
        c.position = Vec3::new(x0 + xd * progress, y, z0 + zd * progress);
        let scale = if ridden { 0.75 } else { 1.0 };
        let max = self.max_speed(c);
        let delta = Vec3::new(
            (scale * c.velocity.x).clamp(-max, max),
            0.0,
            (scale * c.velocity.z).clamp(-max, max),
        );
        self.move_cart(c, delta);
        // 🤖 Agent: `AbstractMinecart.move` runs applyEffectsFromBlocks at
        // once, the cart still at the rail's top height (below it is lowered).
        let here = c.position;
        self.cart_inside_blocks(c, slot, here, here);
        let at = |e: [i32; 3], c: &Minecart| {
            c.position.x.floor() as i32 - pos[0] == e[0]
                && c.position.z.floor() as i32 - pos[2] == e[2]
        };
        if e0[1] != 0 && at(e0, c) {
            c.position.y += f64::from(e0[1]);
        } else if e1[1] != 0 && at(e1, c) {
            c.position.y += f64::from(e1[1]);
        }
        c.velocity = self.natural_slowdown(c, ridden, 0.96);
        if let (Some(new), Some(old)) = (
            self.rail_pos(Some(c.id), c.position.x, c.position.y, c.position.z),
            old,
        ) {
            let speed = (old[1] - new[1]) * 0.05;
            let h = c.horizontal();
            if h > 0.0 {
                c.velocity.x *= (h + speed) / h;
                c.velocity.z *= (h + speed) / h;
            }
            c.position.y = new[1];
        }
        let (xn, zn) = (c.position.x.floor() as i32, c.position.z.floor() as i32);
        if xn != pos[0] || zn != pos[2] {
            let h = c.horizontal();
            c.velocity = Vec3::new(
                h * f64::from(xn - pos[0]),
                c.velocity.y,
                h * f64::from(zn - pos[2]),
            );
        }
        if power {
            let h = c.horizontal();
            if h > 0.01 {
                c.velocity.x += c.velocity.x / h * 0.06;
                c.velocity.z += c.velocity.z / h * 0.06;
            } else if rail.shape == Shape::EastWest {
                if self.conductor(pos[0] - 1, pos[1], pos[2]) {
                    c.velocity.x = 0.02;
                } else if self.conductor(pos[0] + 1, pos[1], pos[2]) {
                    c.velocity.x = -0.02;
                }
            } else if rail.shape == Shape::NorthSouth {
                if self.conductor(pos[0], pos[1], pos[2] - 1) {
                    c.velocity.z = 0.02;
                } else if self.conductor(pos[0], pos[1], pos[2] + 1) {
                    c.velocity.z = -0.02;
                }
            }
        }
    }
    /// `Entity.checkInsideBlocks` for a recorded move of the cart from `from`
    /// to `to`: the entity-sensing blocks (plates, buttons, tripwires, detector
    /// rails) on the path run `entityInside` now, with the cart where the tick
    /// left it. `slot` is where the cart is stored.
    fn cart_inside_blocks(&mut self, c: &mut Minecart, slot: usize, from: Vec3, to: Vec3) {
        let shrink = f64::from(1.0e-5f32);
        let half = WIDTH / 2.0;
        let bounds = Aabb::new(
            Vec3::new(to.x - half, to.y, to.z - half),
            Vec3::new(to.x + half, to.y + HEIGHT, to.z + half),
        )
        .deflate(Vec3::new(shrink, shrink, shrink));
        let mut hits = Vec::new();
        let behaviours = self.res.behaviours();
        let world = &self.world;
        let _ = horizon_core::geometry::sweep_intersected_blocks(
            from,
            to,
            bounds,
            1_000_000,
            |p, _| {
                if behaviours.senses_entities(world.block(p.x, p.y, p.z)) {
                    hits.push(p);
                }
                true
            },
        );
        if hits.is_empty() {
            return;
        }
        let Some(i) = cart_at(&self.carts, c.id, slot) else {
            return;
        };
        // The stored cart is the one a plate counts: it takes the moved cart
        // for the walk and hands back what the walk left.
        self.carts.update(i, |stored| std::mem::swap(stored, c));
        self.with_level(|b, level, ticks| {
            for p in hits {
                b.entity_inside(level, ticks, p);
            }
        });
        self.carts.update(i, |stored| std::mem::swap(stored, c));
        let _ = self.process_level_events();
    }
    /// `AbstractMinecart`: `minecart_improvements` picks the smoother
    /// physics.
    pub(super) fn minecart_improvements(&self) -> bool {
        self.enabled_feature_ids()
            .iter()
            .any(|f| f == "minecraft:minecart_improvements")
    }
    /// `NewMinecartBehavior.getMaxSpeed`: the game rule over water's half,
    /// every cart alike (`cart_max_speed` cuts a furnace minecart's).
    fn max_speed_new(&self, c: &Minecart) -> f64 {
        f64::from(
            self.ui
                .rules
                .int_at(horizon_client::gamerules::vanilla::MAX_MINECART_SPEED),
        ) * if c.in_water { 0.5 } else { 1.0 }
            / 20.0
    }
    /// `AbstractMinecart.getRedstoneDirection` (over the old port's
    /// conductor proxy): which way a powered rail pushes a resting cart.
    fn redstone_direction(&self, pos: [i32; 3], shape: Shape) -> Vec3 {
        let (dx, dz) = match shape {
            Shape::NorthSouth => (0, 1),
            Shape::EastWest => (1, 0),
            _ => return Vec3::ZERO,
        };
        // The push points away from the signal's side (getRedstoneDirection).
        if self.conductor(pos[0] - dx, pos[1], pos[2] - dz) {
            Vec3::new(f64::from(dx), 0.0, f64::from(dz))
        } else if self.conductor(pos[0] + dx, pos[1], pos[2] + dz) {
            Vec3::new(f64::from(-dx), 0.0, f64::from(-dz))
        } else {
            Vec3::ZERO
        }
    }
    /// `NewMinecartBehavior.isDecending`: the movement goes down the
    /// shape's rise.
    fn is_descending(movement: Vec3, shape: Shape) -> bool {
        match shape {
            Shape::AscendingEast => movement.x < 0.0,
            Shape::AscendingWest => movement.x > 0.0,
            Shape::AscendingNorth => movement.z > 0.0,
            Shape::AscendingSouth => movement.z < 0.0,
            _ => false,
        }
    }
    /// `NewMinecartBehavior.setRotation`: preserve the cart's facing when
    /// motion reverses, including the pitch sign on a slope.
    fn set_cart_rotation(c: &mut Minecart, mut yaw: f32, mut pitch: f32) {
        if (175.0..=185.0).contains(&f64::from((yaw - c.yaw).abs())) {
            c.flipped = !c.flipped;
            yaw -= 180.0;
            pitch = -pitch;
        }
        c.yaw = yaw % 360.0;
        c.pitch = pitch.clamp(-45.0, 45.0) % 360.0;
    }
    /// `NewMinecartBehavior.MinecartStep`: wire rotations use ROTATION_BYTE.
    fn record_cart_step(c: &mut Minecart, weight: f32) {
        use horizon_protocol::buf::pack_degrees;
        c.lerp_steps
            .push(horizon_protocol::packets::play::MinecartStep {
                position: c.position.components(),
                movement: c.velocity.components(),
                yaw: pack_degrees(c.yaw),
                pitch: pack_degrees(c.pitch),
                weight,
            });
    }
    /// `NewMinecartBehavior.moveAlongTrack`: each step has its own rotation
    /// and interpolation weight, including the final off-rail move.
    fn finish_cart_step(&self, c: &mut Minecart, old: Vec3, initial: Vec3) {
        let delta = old.vector_to(c.position);
        let length = delta.length();
        if length > f64::from(1.0e-5_f32) {
            if delta.horizontal_distance_squared() > f64::from(1.0e-5_f32) {
                let yaw = 180.0 - java_degrees(delta.z, delta.x) as f32
                    + if c.flipped { 180.0 } else { 0.0 };
                let pitch = if c.on_ground && !c.on_rails {
                    0.0
                } else {
                    (90.0 - java_degrees(delta.horizontal_distance(), delta.y) as f32)
                        * if c.flipped { -1.0 } else { 1.0 }
                };
                Self::set_cart_rotation(c, yaw, pitch);
            } else if !c.on_rails {
                c.pitch = if c.on_ground {
                    0.0
                } else {
                    c.pitch + 0.2 * ((-c.pitch + 180.0).rem_euclid(360.0) - 180.0)
                };
            }
            Self::record_cart_step(c, length.min(self.max_speed_new(c)) as f32);
        } else if initial.horizontal_distance_squared() > 0.0 {
            Self::record_cart_step(c, 1.0);
        }
    }
    /// `NewMinecartBehavior.adjustToRails`: snap only on the first tick or
    /// when entering rails; curves project onto the segment between exits.
    fn adjust_cart_to_rails(c: &mut Minecart, pos: [i32; 3], shape: Shape) {
        let (e0, e1) = shape.exits();
        let exit =
            |e: [i32; 3]| Vec3::new(f64::from(e[0]), f64::from(e[1]), f64::from(e[2])).scale(0.5);
        let old = c.position;
        let (exit0, exit1) = (exit(e0), exit(e1));
        let center = Vec3::new(
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]),
            f64::from(pos[2]) + 0.5,
        );
        let mut out = Vec3::new(exit0.x, 0.0, exit0.z);
        let mut into = Vec3::new(exit1.x, 0.0, exit1.z);
        if c.velocity.length() > f64::from(1.0e-5_f32) && c.velocity.dot(out) < c.velocity.dot(into)
            || Self::is_descending(into, shape)
        {
            std::mem::swap(&mut out, &mut into);
        }
        let mut yaw = 180.0 - java_degrees(out.z, out.x) as f32;
        if exit0.x != exit1.x && exit0.z != exit1.z {
            let along = exit1.subtract(exit0);
            let from = c.position.subtract(center).subtract(exit0);
            let projected = along.scale(along.dot(from) / along.dot(along));
            c.position = center.add_vector(exit0).add_vector(projected);
            yaw = 180.0 - java_degrees(projected.z, projected.x) as f32;
        } else {
            if exit0.z != exit1.z {
                c.position.x = center.x;
            }
            if exit0.x != exit1.x {
                c.position.z = center.z;
            }
            c.position.y = center.y;
        }
        let pitch = if shape.is_slope() {
            c.position.y += center.add_vector(into).distance(c.position) + 0.1;
            if c.flipped { 45.0 } else { -45.0 }
        } else {
            c.position.y += 0.1;
            0.0
        };
        Self::set_cart_rotation(c, yaw + if c.flipped { 180.0 } else { 0.0 }, pitch);
        let distance = old.distance(c.position);
        if distance > 0.0 {
            Self::record_cart_step(c, if c.first_tick { 0.0 } else { distance as f32 });
        }
    }
    /// `NewMinecartBehavior.moveAlongTrack`: the iteration keeps moving
    /// while distance is left, over as many rails as it takes, the speed
    /// recomputed per rail (slope, halt, boost).
    fn move_along_track_new(&mut self, c: &mut Minecart, intent: [f64; 2]) {
        // `TrackIteration`: the slope, halt and boost speeds each apply once
        // per tick, and the iteration keeps moving while distance is left.
        let mut first = true;
        let mut movement_left = 0.0;
        let mut slope_done = false;
        let mut halted = false;
        let mut boosted = false;
        let mut step_start = c.previous;
        while first || movement_left > f64::from(1.0e-5_f32) {
            let initial = c.velocity;
            let pos = self.rail_block(c.position);
            let Some(rail) = self.rail(Some(c.id), pos[0], pos[1], pos[2]) else {
                c.on_rails = false;
                self.come_off_track(c);
                self.finish_cart_step(c, step_start, initial);
                if first || c.position.distance(step_start) > f64::from(1.0e-5_f32) {
                    self.cart_block_effects(c);
                    self.cart_block_effects(c);
                }
                return;
            };
            if !c.on_rails {
                c.on_rails = true;
                Self::adjust_cart_to_rails(c, pos, rail.shape);
            }
            c.fall_distance = 0.0;
            step_start = c.position;
            self.note_cart_pass(c, pos);
            if &*rail.name == "minecraft:activator_rail" {
                self.activate_cart(c, rail.powered);
            }
            let ridden = self.cart_is_vehicle(c.id);
            let initial_h = c.velocity.horizontal_distance();
            let mut v = Vec3::new(c.velocity.x, 0.0, c.velocity.z);
            // calculateSlopeSpeed (once per tick): the slide grows with the
            // speed, so a descending cart keeps its pace.
            if !slope_done {
                let slide =
                    0.0078125_f64.max(initial_h * 0.02) * if c.in_water { 0.2 } else { 1.0 };
                let before = v.horizontal_distance_squared();
                match rail.shape {
                    Shape::AscendingEast => v.x -= slide,
                    Shape::AscendingWest => v.x += slide,
                    Shape::AscendingNorth => v.z += slide,
                    Shape::AscendingSouth => v.z -= slide,
                    _ => {}
                }
                slope_done = v.horizontal_distance_squared() != before;
                if !slope_done {
                    v = Vec3::new(c.velocity.x, 0.0, c.velocity.z);
                }
            }
            // calculatePlayerInputSpeed (first iteration).
            if first
                && ridden
                && (intent[0] != 0.0 || intent[1] != 0.0)
                && v.x * v.x + v.z * v.z < 0.01
            {
                let input = Vec3::new(intent[0], 0.0, intent[1])
                    .normalize()
                    .scale(0.001);
                let pushed = v.add_vector(input);
                if pushed.horizontal_distance_squared() != v.horizontal_distance_squared() {
                    v = pushed;
                    halted = true;
                }
            }
            // calculateHaltTrackSpeed (until the input halted).
            if !halted && &*rail.name == "minecraft:powered_rail" && !rail.powered {
                let before = v.x.hypot(v.z);
                if before < 0.03 {
                    v = Vec3::ZERO;
                } else {
                    v = Vec3::new(v.x * 0.5, v.y, v.z * 0.5);
                }
                halted = v.x.hypot(v.z) != before;
            }
            // applyNaturalSlowdown and the speed cap (first iteration).
            if first {
                c.velocity = v;
                v = self.natural_slowdown(c, ridden, 0.975);
                if v.length_squared() > 0.0 {
                    let speed = v.length().min(self.cart_max_speed(c));
                    v = v.normalize().scale(speed);
                }
            }
            // calculateBoostTrackSpeed (once per tick).
            if !boosted && &*rail.name == "minecraft:powered_rail" && rail.powered {
                let before = v;
                let len = v.length();
                if len > 0.01 {
                    v = v.normalize().scale(len + 0.06);
                } else {
                    let dir = self.redstone_direction(pos, rail.shape);
                    if dir != Vec3::ZERO {
                        v = dir.scale(len + 0.2);
                    }
                }
                boosted = v.horizontal_distance_squared() != before.horizontal_distance_squared();
                if !boosted {
                    v = before;
                }
            }
            c.velocity = v;
            let new_h = v.horizontal_distance();
            movement_left = if first {
                new_h
            } else {
                movement_left + (new_h - initial_h)
            };
            movement_left = self.new_step_along_track(c, pos, &rail, movement_left);
            self.finish_cart_step(c, step_start, initial);
            // NewMinecartBehavior.moveAlongTrack applies block effects twice,
            // after recording the step, including a stationary first step.
            if first || c.position.distance(step_start) > f64::from(1.0e-5_f32) {
                self.cart_block_effects(c);
                self.cart_block_effects(c);
            }
            first = false;
        }
    }
    /// `NewMinecartBehavior.stepAlongTrack`: toward the rail's exit, the
    /// hill's Y projected, a V of opposing slopes resting a slow cart.
    fn new_step_along_track(
        &mut self,
        c: &mut Minecart,
        pos: [i32; 3],
        rail: &Rail,
        mut movement_left: f64,
    ) -> f64 {
        if movement_left < f64::from(1.0e-5_f32) {
            return 0.0;
        }
        let old = c.position;
        let (e0, e1) = rail.shape.exits();
        let movement = Vec3::new(c.velocity.x, 0.0, c.velocity.z);
        if movement.length() < f64::from(1.0e-5_f32) {
            c.velocity = Vec3::ZERO;
            return 0.0;
        }
        let in_hill = e0[1] != e1[1];
        let mut out_dir = Vec3::new(f64::from(e0[0]) * 0.5, 0.0, f64::from(e0[2]) * 0.5);
        let in_dir = Vec3::new(f64::from(e1[0]) * 0.5, 0.0, f64::from(e1[2]) * 0.5);
        if movement.dot(out_dir) < movement.dot(in_dir) {
            out_dir = in_dir;
        }
        let out_len = out_dir.x.hypot(out_dir.z);
        let mut out_pos = Vec3::new(
            f64::from(pos[0]) + 0.5 + out_dir.x + out_dir.x / out_len * f64::from(1.0e-5_f32),
            f64::from(pos[1]) + 0.1,
            f64::from(pos[2]) + 0.5 + out_dir.z + out_dir.z / out_len * f64::from(1.0e-5_f32),
        );
        if in_hill && !Self::is_descending(c.velocity, rail.shape) {
            out_pos.y += 1.0;
        }
        let towards_out = c.position.vector_to(out_pos).normalize();
        // The movement turns onto the exit's direction, at the same speed
        // (`this.setDeltaMovement(movement)` at stepAlongTrack's end).
        let h_speed = movement.x.hypot(movement.z);
        let th = towards_out.x.hypot(towards_out.z);
        let movement = towards_out.scale(h_speed / th);
        let mut new_pos = if in_hill {
            old.add_vector(
                movement
                    .normalize()
                    .scale(movement_left * f64::from(std::f32::consts::SQRT_2)),
            )
        } else {
            old.add_vector(movement.normalize().scale(movement_left))
        };
        if out_pos.distance(old) <= new_pos.distance(old) {
            movement_left = out_pos.vector_to(new_pos).horizontal_distance();
            new_pos = out_pos;
        } else {
            movement_left = 0.0;
        }
        self.move_cart(c, old.vector_to(new_pos));
        if in_hill {
            let new_block = [
                new_pos.x.floor() as i32,
                new_pos.y.floor() as i32,
                new_pos.z.floor() as i32,
            ];
            if let Some(new_rail) = self.rail(Some(c.id), new_block[0], new_block[1], new_block[2])
            {
                // restAtVShape: opposing slopes rest a slow cart.
                if c.velocity.length_squared() < 0.005
                    && new_rail.shape.is_slope()
                    && Self::is_descending(c.velocity, rail.shape)
                    && !Self::is_descending(c.velocity, new_rail.shape)
                {
                    c.velocity = Vec3::ZERO;
                    return 0.0;
                }
            }
            let h_dist = (out_pos.x - c.position.x).hypot(out_pos.z - c.position.z);
            let project = out_pos.y
                + if Self::is_descending(movement, rail.shape) {
                    h_dist
                } else {
                    -h_dist
                };
            if c.position.y < project {
                c.position.y = project;
            }
        }
        // The cart did not move but the step would have: it is stuck.
        if c.position.distance(old) < f64::from(1.0e-5_f32)
            && new_pos.distance(old) > f64::from(1.0e-5_f32)
        {
            c.velocity = Vec3::ZERO;
            return 0.0;
        }
        c.velocity = movement;
        movement_left
    }
    /// `comeOffTrack`.
    fn come_off_track(&self, c: &mut Minecart) {
        let max = self.cart_max_speed(c);
        c.velocity.x = c.velocity.x.clamp(-max, max);
        c.velocity.z = c.velocity.z.clamp(-max, max);
        if c.on_ground {
            c.velocity = Vec3::new(c.velocity.x * 0.5, c.velocity.y * 0.5, c.velocity.z * 0.5);
        }
        let v = c.velocity;
        self.move_cart(c, v);
        // Entity.move's last step: soul sand and honey slow a derailed cart.
        // The rail paths set the track's velocity again after their move, so
        // the factor changes nothing there (measured on a rail over soul sand).
        let factor = crate::motion::block_speed_factor_at(
            &self.world,
            &self.motion.blocks,
            &self.loop_context,
            c.position,
            None,
        );
        c.velocity = horizon_world::entity_move::scale_by_block_speed(c.velocity, factor);
        if !c.on_ground {
            // scale(getAirDrag()): 0.95F, widened.
            let drag = f64::from(0.95_f32);
            c.velocity = Vec3::new(
                c.velocity.x * drag,
                c.velocity.y * drag,
                c.velocity.z * drag,
            );
        }
    }
    /// `applyEffectsFromBlocks` for a cart: a bubble column lifts or drags it
    /// and resets the fall distance.
    pub(super) fn cart_block_effects(&self, c: &mut Minecart) {
        if c.push_by_bubble_column(
            &self.world,
            self.pack.assets.registry,
            self.res.facts(),
            self.collision,
        ) {
            c.fall_distance = 0.0;
        }
        // PowderSnowBlock's FREEZE contact is counted here too.
        // WebBlock.entityInside: makeStuckInBlock(0.25, 0.05, 0.25) for every
        // cobweb the box (deflated by 1.0E-5) is in; it resets the fall distance.
        let b = c.bounds().deflate(Vec3::new(1e-5, 1e-5, 1e-5));
        let (lo, hi) = (b.min_position(), b.max_position());
        let facts = self.res.facts();
        let mut froze = false;
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    let name = &*facts.get(self.world.block(x, y, z)).name;
                    if name == "minecraft:powder_snow" {
                        froze = true;
                    }
                    if name == "minecraft:cobweb" {
                        c.fall_distance = 0.0;
                        c.stuck = Vec3::new(0.25, f64::from(0.05_f32), 0.25);
                    }
                }
            }
        }
        if froze {
            // InsideBlockEffectType.FREEZE: once per applyEffectsFromBlocks.
            c.ticks_frozen = (c.ticks_frozen + 1)
                .min(horizon_client::motion::PlayerMotion::TICKS_REQUIRED_TO_FREEZE);
        }
    }
    /// Runs `push_from_cart` for the cart being ticked (`c` is a copy of the
    /// stored one, kept at `slot`): the stored cart is replaced by `c` for the
    /// push and `c` takes back what the push changed. The rolled loot stays
    /// with `c`: the stored cart is without it for the whole tick
    /// (`tick_carts_now`), and a mineshaft's chest cart carries its whole roll
    /// as NBT.
    fn push_from_ticked_cart(&mut self, c: &mut Minecart, slot: usize, index: &mut CartIndex) {
        let Some(i) = cart_at(&self.carts, c.id, slot) else {
            return;
        };
        let loot = c.loot.take();
        if !self.store_cart_back(i, c.clone()) {
            return;
        }
        self.push_from_cart(i, index);
        if let Some(i) = self.carts.iter().position(|x| x.id == c.id) {
            *c = self.carts[i].clone();
        }
        c.loot = loot;
    }
    /// `AbstractMinecart.tick` for one cart; returns false once it is gone.
    /// `slot` is where the cart is stored (where to look first).
    fn tick_cart(
        &mut self,
        c: &mut Minecart,
        intent: [f64; 2],
        slot: usize,
        index: &mut CartIndex,
    ) -> bool {
        c.tick_count += 1;
        c.previous = c.position;
        c.previous_yaw = c.yaw;
        c.previous_pitch = c.pitch;
        // AbstractMinecart.tick: each counts down only while positive.
        if c.hurt_time > 0 {
            c.hurt_time -= 1;
        }
        if c.damage > 0.0 {
            c.damage -= 1.0;
        }
        // NewMinecartBehavior.tick snaps a newly created cart before gravity.
        if self.minecart_improvements() && c.first_tick {
            let pos = self.rail_block(c.position);
            if let Some(rail) = self.rail(Some(c.id), pos[0], pos[1], pos[2]) {
                c.on_rails = true;
                Self::adjust_cart_to_rails(c, pos, rail.shape);
            } else {
                c.on_rails = false;
            }
        }
        // applyGravity
        c.velocity.y -= c.get_gravity();
        let pos = self.rail_block(c.position);
        if self.minecart_improvements() {
            self.move_along_track_new(c, intent);
        } else if let Some(rail) = self.rail(Some(c.id), pos[0], pos[1], pos[2]) {
            self.move_along_track(c, pos, &rail, intent, slot);
            self.note_cart_pass(c, pos);
            if &*rail.name == "minecraft:activator_rail" {
                self.activate_cart(c, rail.powered);
            }
        } else {
            self.come_off_track(c);
        }
        c.first_tick = false;
        // The old behavior derives yaw once per tick; the experiment has
        // already recorded each step's yaw and pitch.
        if !self.minecart_improvements() {
            // OldMinecartBehavior.tick: setXRot(0.0F) before the yaw update.
            c.pitch = 0.0;
            let (xd, zd) = (c.previous.x - c.position.x, c.previous.z - c.position.z);
            if xd * xd + zd * zd > 0.001 {
                c.yaw = horizon_core::geometry::trig::atan2_degrees_double(zd, xd);
                if c.flipped {
                    c.yaw += 180.0;
                }
            }
            let diff = (c.yaw - c.previous_yaw + 180.0).rem_euclid(360.0) - 180.0;
            if !(-170.0..170.0).contains(&diff) {
                c.yaw += 180.0;
                c.flipped = !c.flipped;
            }
            c.yaw %= 360.0;
            c.pitch %= 360.0;
        }
        if !self.minecart_improvements() {
            self.cart_block_effects(c);
            // OldMinecartBehavior.tick: applyEffectsFromBlocks() once settled.
            let here = c.position;
            self.cart_inside_blocks(c, slot, here, here);
        }
        // OldMinecartBehavior.tick ends with pushAndPickupEntities, before
        // AbstractMinecart.tick's updateFluidInteraction. The experiment still
        // uses this fallback until its per-move pickup/push retry is ported.
        self.push_from_ticked_cart(c, slot, index);
        // AbstractMinecart.tick: updateFluidInteraction, a stream pushes the cart.
        c.in_water = c
            .push_by_fluid(&self.world, &self.res.motion().fluids, &self.loop_context)
            .0
            > 0.0;
        // updateFluidInteraction: being in water resets the fall distance.
        if c.in_water {
            c.fall_distance = 0.0;
        }
        // Entity.checkInsideBlocks runs once after the cart tick, sweeping every
        // movement through powder snow. PowderSnowBlock uses getKnownMovement,
        // which is getDeltaMovement for a minecart.
        self.entity_powder_snow_particles(c.previous, c.position, c.bounds(), c.velocity, false);
        // Variants.
        match c.kind {
            CartKind::Furnace => {
                if c.fuel > 0 {
                    c.fuel -= 1;
                }
                if c.fuel <= 0 {
                    c.push = Vec3::ZERO;
                }
                if c.fuel > 0 && c.random.next_int_bound(4).unwrap_or(1) == 0 {
                    let at = [c.position.x, c.position.y + 0.8, c.position.z];
                    self.particles.add(
                        &ParticleOptions::simple(ParticleType::LargeSmoke),
                        at,
                        [0.0; 3],
                    );
                }
            }
            CartKind::Tnt => {
                // MinecartTNT.causeFallDamage ran inside the move.
                if let Some(speed_sqr) = c.fall_blast.take()
                    && self.explode_cart(c, speed_sqr)
                {
                    return false;
                }
                // MinecartTNT.tick: the fuse and the collision check are separate.
                if c.fuse > 0 {
                    c.fuse -= 1;
                    let at = [c.position.x, c.position.y + 0.5, c.position.z];
                    self.particles
                        .add(&ParticleOptions::simple(ParticleType::Smoke), at, [0.0; 3]);
                } else if c.fuse == 0 {
                    // explode(horizontalDistanceSqr()): a rising cart's
                    // vertical speed adds nothing.
                    let speed = c.velocity.x * c.velocity.x + c.velocity.z * c.velocity.z;
                    if self.explode_cart(c, speed) {
                        return false;
                    }
                }
                // MinecartTNT.tick reads the delta left after AbstractMinecart.tick
                // (pushes and fluids included). The move zeroed the blocked axes
                // (Entity.restituteMovementAfterCollisions) and no track step puts
                // them back: a head-on stop leaves nothing, a diagonal one keeps
                // the free axis (#11380).
                let speed = c.velocity.horizontal_distance_squared();
                if tnt_ignites_by_collision(c.horizontal_collision, speed)
                    && self.explode_cart(c, speed)
                {
                    return false;
                }
            }
            // MinecartSpawner.tick: `BaseSpawner.serverTick` at the cart's block.
            CartKind::Spawner => {
                if let Some(mut spawner) = c.spawner.take() {
                    let key = (
                        c.position.x.floor() as i32,
                        c.position.y.floor() as i32,
                        c.position.z.floor() as i32,
                    );
                    let _ = self.server_tick_spawner_of(key, &mut spawner, Some(c.id));
                    c.spawner = Some(spawner);
                }
            }
            _ => {}
        }
        true
    }
    /// The cart passes over the rail at `pos`: `minecart_pass`, delivered after its batch.
    fn note_cart_pass(&self, c: &Minecart, pos: [i32; 3]) {
        horizon_client::block_contact::minecart_pass(
            c.id as i32,
            self.world.block(pos[0], pos[1], pos[2]),
            BlockPos::new(pos[0], pos[1], pos[2]),
        );
    }
    /// `activateMinecart` on an activator rail.
    fn activate_cart(&mut self, c: &mut Minecart, powered: bool) {
        match c.kind {
            CartKind::Rideable if powered => {
                // Minecart.activateMinecart: ejectPassengers.
                if self.riding == Some(c.id)
                    && self.native_may_leave(c.id)
                    && self.riding == Some(c.id)
                {
                    self.riding = None;
                }
                for (_, rider, player) in self.seated_passengers(c.id) {
                    if !player
                        && self
                            .mob_object(rider)
                            .is_some_and(|mob| !mob.removed && mob.extra.mount == Some(c.id))
                    {
                        self.dismount_mob_from_cart(rider, c);
                    }
                }
                if c.hurt_time == 0 {
                    c.hurt_dir = -c.hurt_dir;
                    c.hurt_time = 10;
                    c.damage = 50.0;
                }
            }
            CartKind::Tnt if powered && c.fuse < 0 => self.prime_cart(c),
            CartKind::Hopper => c.enabled = !powered,
            // MinecartCommandBlock.activateMinecart: the command runs at most
            // every ACTIVATION_DELAY ticks.
            CartKind::CommandBlock if powered && c.tick_count - c.last_activated >= 4 => {
                c.command_due = true;
                c.last_activated = c.tick_count;
            }
            _ => {}
        }
    }
    /// `MinecartTNT.primeFuse`: fuse 80, then `Level.broadcastEntityEvent`
    /// (`EntityEvent.TNT_PRIME`, 70) and, unless the cart is silent,
    /// `SoundEvents.TNT_PRIMED` on `SoundSource.BLOCKS`. A client that
    /// receives 70 only sets the fuse (`handleEntityEvent`).
    pub(super) fn prime_cart(&mut self, c: &mut Minecart) {
        if !self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::TNT_EXPLODES)
        {
            return;
        }
        c.fuse = 80;
        let at = [c.position.x, c.position.y, c.position.z];
        let id = c.id as i32;
        self.lan_entity_event(id, TNT_PRIME_EVENT, at);
        if !self.entity_silent(c.id) {
            self.audio
                .play_on(BLOCKS, "minecraft:entity.tnt.primed", Some(at), 1.0, 1.0);
        }
    }
    /// `MinecartTNT.handleEntityEvent`: event 70 calls `primeFuse`, which on a
    /// client only sets the fuse to 80. The window then counts it down
    /// (`MinecartTNT.tick`) for the smoke, the flash and the swell.
    pub(super) fn remote_tnt_cart_event(&mut self, id: i32, event: i8) {
        if event != TNT_PRIME_EVENT {
            return;
        }
        if let Some(i) = self
            .carts
            .iter()
            .position(|c| c.id == id as u32 && c.kind == CartKind::Tnt)
        {
            self.carts.update(i, |c| c.fuse = 80);
        }
    }
    /// `AbstractMinecart.push` from each player walking into a cart
    /// (`LivingEntity.pushEntities`): the window's player and every server
    /// player (single player's own on its server thread, LAN guests). A
    /// player in a vehicle pushes nothing here. The quarter push the player
    /// gets back only moves the server's copy of it (`Entity.push` marks
    /// `needsSync`, which `ServerEntity` sends to the other players only),
    /// so no player feels it.
    fn push_carts(&mut self) {
        if self.carts.is_empty() {
            return;
        }
        // (feet, box)
        let mut pushers: Vec<(Vec3, Aabb)> = Vec::new();
        if self.riding.is_none()
            && let Some(p) = self.player.as_ref()
        {
            pushers.push((p.position, p.bounds()));
        }
        if let Some(host) = self.lan.as_ref() {
            for g in host.hub.guests.iter().filter(|g| {
                host.hub.here(g) && g.loaded && !g.dead && !g.spectator() && g.vehicle.is_none()
            }) {
                let [x, y, z] = g.position;
                pushers.push((Vec3::new(x, y, z), g.bounds()));
            }
        }
        let cells =
            horizon_client::mobs::seen_grid::SeenGrid::new(self.carts.iter().map(Minecart::bounds));
        let mut near = Vec::new();
        for (feet, bb) in pushers {
            if !cells.candidates(bb, &mut near) {
                near.extend(0..self.carts.len() as u32);
            }
            for &k in &near {
                let k = k as usize;
                self.carts.update(k, |c| {
                    if !bb.intersects(c.bounds()) {
                        return;
                    }
                    let (mut xa, mut za) = (feet.x - c.position.x, feet.z - c.position.z);
                    let dd = xa * xa + za * za;
                    if dd < f64::from(1.0e-4f32) {
                        return;
                    }
                    let d = dd.sqrt();
                    let pow = (1.0 / d).min(1.0);
                    xa = xa / d * pow * f64::from(0.1f32) * 0.5;
                    za = za / d * pow * f64::from(0.1f32) * 0.5;
                    c.velocity.x -= xa;
                    c.velocity.z -= za;
                });
            }
        }
        self.push_carts_from_mobs(&cells);
    }
    /// `AbstractMinecart.push(mob)` from each mob walking into a cart (`LivingEntity.pushEntities`
    /// -> `doPush`): the cart gets the push and the mob a quarter of it back (`Entity.push`, which
    /// needs the mob to be pushable and to carry no one). Bats push nothing (`Bat.pushEntities`),
    /// nor does a mob whose box reaches no cart: its box is worked out only if it may.
    fn push_carts_from_mobs(&mut self, cells: &SeenGrid) {
        let (teams, mut near) = (self.mob_teams(), Vec::new());
        // Every mob, each asked which carts it is near: a mob far from all of them is turned
        // away by a bitmap of the carts' coarse cells, without a lookup or a grid of the mobs.
        let reach = self.vehicle_mob_reach();
        let coarse = reach.map(|_| cells::CartCells::of(self.carts.iter().map(Minecart::bounds)));
        // (mob index, feet, box)
        let pushers: Vec<(usize, Vec3, Aabb)> = (0..self.mobs.len() as u32)
            .into_iter()
            .map(|j| (j as usize, &self.mobs[j as usize]))
            .filter(|(_, m)| {
                // The test that turns most mobs away comes first: it reads only the position.
                reach.is_none_or(|r| {
                    coarse
                        .as_ref()
                        .is_none_or(|c| c.may_meet(r.around(m.position)))
                }) && !m.removed
                    && m.alive()
                    && m.species() != Species::Bat
                    && reach.is_none_or(|r| self.mob_may_meet_cart(r, m, cells, &mut near))
                    && pushable_by(teams.of(Who::Mob(m.id)), None, true)
            })
            .map(|(j, m)| (j, m.position, m.bounds()))
            .collect();
        drop(teams);
        for (j, feet, bb) in pushers {
            if !cells.candidates(bb, &mut near) {
                near.extend(0..self.carts.len() as u32);
            }
            for &k in &near {
                let k = k as usize;
                self.carts.update(k, |c| {
                    // hasPassenger(mob): a mob in this cart never pushes it.
                    if self.mobs[j].extra.mount == Some(c.id) || !bb.intersects(c.bounds()) {
                        return;
                    }
                    let Some((xa, za)) = cart_push_from(feet, c.position) else {
                        return;
                    };
                    c.velocity.x -= xa;
                    c.velocity.z -= za;
                    let m = &mut self.mobs[j];
                    if m.pushable() && m.rider.is_none() && m.extra.rider.is_none() {
                        m.velocity.x += xa / 4.0;
                        m.velocity.z += za / 4.0;
                        m.needs_sync = true;
                    }
                });
            }
        }
    }
    /// `OldMinecartBehavior.pushAndPickupEntities` for carts, at the end of
    /// cart `i`'s own tick: each other cart in its box (0.2 wider on each
    /// side) gets `push(cart i)`. Running it per cart, after that cart moved
    /// and before the next one does, is what splits stacked carts apart.
    fn push_from_cart(&mut self, i: usize, index: &mut CartIndex) {
        let reach = f64::from(0.2f32);
        let hitbox = self.carts[i].bounds().inflate(Vec3::new(reach, 0.0, reach));
        let mut near = std::mem::take(&mut index.near);
        if !index.carts_near(&self.carts, i, hitbox, &mut near) {
            near.clear();
            near.extend(0..self.carts.len() as u32);
        }
        let allowed = self.cart_push_allowed(&self.carts, i, &near);
        for (k, &j) in near.iter().enumerate() {
            let j = j as usize;
            if !allowed[k] || j == i || !hitbox.intersects(self.carts[j].bounds()) {
                continue;
            }
            // AbstractMinecart.push(entity): `this` is cart j, `entity` cart i.
            let (this, other) = (&self.carts[j], &self.carts[i]);
            let (mut xa, mut za) = (
                other.position.x - this.position.x,
                other.position.z - this.position.z,
            );
            let dd = xa * xa + za * za;
            if dd < f64::from(1.0e-4f32) {
                continue;
            }
            let d = dd.sqrt();
            xa /= d;
            za /= d;
            let pow = (1.0 / d).min(1.0);
            xa = xa * pow * f64::from(0.1f32) * 0.5;
            za = za * pow * f64::from(0.1f32) * 0.5;
            // pushOtherMinecart (old movement): only along this cart's facing.
            let dir = glam::DVec2::new(
                other.position.x - this.position.x,
                other.position.z - this.position.z,
            )
            .normalize_or_zero();
            let r = f64::from(this.yaw * 0.017453292f32);
            let facing = glam::DVec2::new(f64::from(trig::cos(r)), f64::from(trig::sin(r)))
                .normalize_or_zero();
            if dir.dot(facing).abs() < f64::from(0.8f32) {
                continue;
            }
            let (mt, mo) = (this.velocity, other.velocity);
            let (ft, fo) = (
                this.kind == CartKind::Furnace,
                other.kind == CartKind::Furnace,
            );
            let (nt, no) = if fo && !ft {
                (
                    Vec3::new(mt.x * 0.2 + (mo.x - xa), mt.y, mt.z * 0.2 + (mo.z - za)),
                    Vec3::new(mo.x * 0.95, mo.y, mo.z * 0.95),
                )
            } else if !fo && ft {
                (
                    Vec3::new(mt.x * 0.95, mt.y, mt.z * 0.95),
                    Vec3::new(mo.x * 0.2 + (mt.x + xa), mo.y, mo.z * 0.2 + (mt.z + za)),
                )
            } else {
                let (xdd, zdd) = ((mo.x + mt.x) / 2.0, (mo.z + mt.z) / 2.0);
                (
                    Vec3::new(mt.x * 0.2 + (xdd - xa), mt.y, mt.z * 0.2 + (zdd - za)),
                    Vec3::new(mo.x * 0.2 + (xdd + xa), mo.y, mo.z * 0.2 + (zdd + za)),
                )
            };
            self.carts.update(j, |c| c.velocity = nt);
            self.carts.update(i, |c| c.velocity = no);
        }
        index.near = near;
        self.pickup_from_cart(i, index);
    }
    /// `OldMinecartBehavior.pushAndPickupEntities` for living entities in cart
    /// `i`'s box (0.2 wider on each side): an eligible mob boards a moving
    /// rideable cart (`startRiding`); others get `Entity.push`.
    fn pickup_from_cart(&mut self, i: usize, index: &mut CartIndex) {
        let v = self.carts[i].velocity;
        if self.carts[i].kind != CartKind::Rideable || v.x * v.x + v.z * v.z < 0.01 {
            return;
        }
        let reach = f64::from(0.2f32);
        let id = self.carts[i].id;
        let pos = self.carts[i].position;
        let hitbox = self.carts[i].bounds().inflate(Vec3::new(reach, 0.0, reach));
        let mut near = std::mem::take(&mut index.near);
        let mounted = index.mobs_near(&self.mobs, id, hitbox, &mut near);
        let mut occupied = self.cart_is_vehicle(id);
        if mounted.is_none() {
            near.clear();
            near.extend(0..self.mobs.len() as u32);
        }
        // The box first: it turns away nearly every mob of the level.
        near.retain(|&j| {
            let m = &self.mobs[j as usize];
            !m.removed && m.extra.mount != Some(id) && hitbox.intersects(m.bounds())
        });
        let candidates: Vec<u32> = if near.is_empty() {
            Vec::new()
        } else {
            let (teams, own) = (self.mob_teams(), self.team_with_owner(id, || None));
            near.iter()
                .map(|&j| &self.mobs[j as usize])
                .filter(|m| {
                    offered_pickup(m.pushable(), m.species(), m.alive())
                        && pushable_by(own, teams.of(Who::Mob(m.id)), true)
                })
                .map(|m| m.id)
                .collect()
        };
        index.near = near;
        for mob in candidates {
            // By id: a mod's mount hook may have removed mobs since the list
            // was made, which moves the others' indices.
            let Some(j) = self.mobs.iter().position(|m| m.id == mob) else {
                continue;
            };
            let species = self.mobs[j].species();
            let player = &*self.mobs[j].kind.id == "minecraft:player";
            let cooldown = self.mobs[j].extra.boarding_cooldown;
            let passenger = self.mobs[j].extra.mount.is_some();
            if pickup_instead_of_push(species, passenger, occupied) && !player {
                // Entity.startRiding: a mod may cancel it (EntityMountEvent).
                if can_ride_cart(species, cooldown)
                    && self.native_mob_may_ride(mob, id, true)
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob)
                {
                    horizon_client::mobs::shulker::on_start_riding(m);
                    m.extra.mount = Some(id);
                    m.extra.passenger = true;
                    m.ai.nav.stop();
                    m.needs_sync = true;
                    occupied = true;
                    self.seat_mob(mob);
                    self.break_leash(mob, true, false);
                    self.note_started_riding(id);
                    self.vehicle_picked_up();
                }
                continue;
            }
            if !takes_push_from_cart(species) {
                continue;
            }
            let other = self.mobs[j].position;
            let Some(v) = super::boat_passengers::push_impulse(pos, other) else {
                continue;
            };
            if !occupied {
                self.carts
                    .update(i, |c| c.velocity = c.velocity.subtract(v));
            }
            let m = &mut self.mobs[j];
            if m.pushable() {
                m.velocity = m.velocity.add_vector(v);
                m.needs_sync = true;
            }
        }
    }
    /// `positionRider` for the mobs in minecarts: each sits on the cart's
    /// passenger attachment, less its own vehicle attachment.
    pub(super) fn seat_cart_mobs(&mut self, riding: &super::passengers::Riders) {
        for i in 0..self.carts.len() {
            let id = self.carts[i].id;
            let Some(js) = riding.get(&id) else {
                continue;
            };
            let (pos, prev) = (self.carts[i].position, self.carts[i].previous);
            let cart_box = self.carts[i].bounds();
            let seats: Vec<(usize, f64, f64)> = js
                .iter()
                .map(|&j| {
                    let m = &self.mobs[j];
                    // Minecart's box is 0.98 wide.
                    let attachment = riders::vehicle_attachment(m, 0.98);
                    (j, attachment, cart_seat_offset(m.species(), attachment))
                })
                .collect();
            for (j, attachment, offset) in seats {
                let m = &mut self.mobs[j];
                m.seated(id, attachment, cart_box);
                m.position = cart_seat(pos, offset);
                m.previous = cart_seat(prev, offset);
                m.velocity = Vec3::ZERO;
                m.fall_distance = 0.0;
                m.on_ground = false;
                m.extra.passenger = true;
                m.ai.nav.stop();
            }
        }
    }
    /// `ejectPassengers` for the mobs in cart `i`.
    pub(super) fn eject_cart_mobs(&mut self, i: usize) {
        let (id, y) = (self.carts[i].id, self.carts[i].position.y);
        for (_, rider, player) in self.seated_passengers(id) {
            if player
                || self
                    .mob_object(rider)
                    .is_none_or(|mob| mob.removed || mob.extra.mount != Some(id))
            {
                continue;
            }
            self.dismount_cart_mob_at(rider, y);
        }
    }
    /// Refuse a removed target before carrying its owning slot into a tick clone.
    pub(super) fn store_cart_back(&mut self, index: usize, cart: Minecart) -> bool {
        !self
            .persist
            .removals
            .contains(&super::commands::Target::Cart(cart.id))
            && self.carts.store_back(index, cart)
    }

    /// Every cart but the ridden one (which ticks with its rider).
    pub(super) fn tick_carts(&mut self) {
        self.vehicles_ticking(true);
        self.tick_carts_now();
        self.vehicles_ticking(false);
    }
    fn tick_carts_now(&mut self) {
        self.push_carts();
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        let mut round = self.native_tick_round();
        // A server player's minecart moves with its rider (lan_vehicles.rs).
        let ridden = self.guest_vehicles();
        // By id: an exploding cart can destroy or prime others mid-loop.
        let ids: Vec<u32> = self.carts.iter().map(|c| c.id).collect();
        let mut index = CartIndex::default();
        let mut hint = 0;
        for &id in &ids {
            let Some(i) = cart_at(&self.carts, id, hint) else {
                continue;
            };
            hint = i + 1;
            let p = self.carts[i].position;
            if Some(id) == self.riding
                || ridden.contains(&id)
                || !ticking(p.x.floor() as i32, p.z.floor() as i32)
            {
                continue;
            }
            let kind = self.carts[i].kind.id();
            horizon_threads::work::set_entity(horizon_threads::work::entity_id(kind));
            if !self.native_tick_pre_one(&mut round, id, kind) {
                continue;
            }
            let Some(i) = self.cart_in_world(id, i, &round, kind) else {
                continue;
            };
            // The ticked copy takes the loot roll instead of copying it: only
            // opening the cart, a command or a save reads it, never a tick, and
            // a mineshaft's chest cart carries its whole roll as NBT. The
            // stored cart keeps its slots for whatever reads them meanwhile.
            let (due, section_step) =
                self.with_entity_held_out(id, |game| section_steps::tick(game, id, i, &mut index));
            section_steps::tell(self, id, section_step);
            if due {
                self.perform_cart_command(id);
                index.forget_carts();
            }
            // After the ticked cart is stored: a mod reads and changes the cart as it is now.
            self.native_tick_post_one(&round, id, kind);
        }
        horizon_threads::work::clear_work();
        // On the server for every cart, the ridden ones too.
        let mut hint = 0;
        for id in ids {
            if round.canceled(id) {
                continue;
            }
            if let Some(i) = cart_at(&self.carts, id, hint) {
                hint = i + 1;
                let p = self.carts[i].position;
                if ticking(p.x.floor() as i32, p.z.floor() as i32) {
                    self.cart_lava_tick(id, i);
                }
            }
        }
    }
    /// `lavaHurt` twice a tick in lava: from `LavaFluid.entityInside` in the
    /// behaviour's `applyEffectsFromBlocks`, then from `AbstractMinecart.tick`'s
    /// own `isInLava`. The second breaks the cart, or primes a TNT minecart
    /// (`MinecartTNT.destroy` for a fire source). A cart has no `baseTick`,
    /// so the fire `lavaIgnite` lights never burns it.
    fn cart_lava_tick(&mut self, id: u32, slot: usize) {
        for _ in 0..2 {
            let Some(i) = cart_at(&self.carts, id, slot) else {
                return;
            };
            if !self.in_lava(self.carts[i].bounds()) {
                return;
            }
            let p = self.carts[i].position;
            let kind = self.carts[i].kind.id();
            let source = horizon_client::health::HurtSource {
                damage_type: "minecraft:lava",
                causing: 0,
                direct: 0,
                position: None,
            };
            let vanilla = self.saved_refuses_damage(id, "minecraft:lava", false);
            if !self.native_nonliving_invulnerable(id, kind, source, vanilla) {
                self.hurt_cart(i, 4.0, false, true, CartHit::default());
            }
            self.lava_hurt_sound([p.x, p.y, p.z]);
        }
    }
    /// `ClientPacketListener.postAddEntitySoundInstance` for the carts that
    /// joined the level since the last tick, and `LocalPlayer.startRiding`'s
    /// two inside sounds when the player got in one; then what the sound
    /// instances read this tick (the ridden cart and every cart).
    pub(super) fn start_cart_sounds(&mut self) -> (Option<u32>, Vec<CartSound>) {
        let carts: Vec<CartSound> = self
            .carts
            .iter()
            .map(|c| CartSound {
                id: c.id,
                position: [c.position.x, c.position.y, c.position.z],
                // Vec3.horizontalDistance.
                speed: (c.velocity.x * c.velocity.x + c.velocity.z * c.velocity.z).sqrt(),
            })
            .collect();
        self.cart_sounds
            .retain(|id| carts.iter().any(|c| c.id == *id));
        for cart in &carts {
            if !self.cart_sounds.contains(&cart.id) {
                let instance = SoundInstance::minecart(cart, random_source::create());
                let (_, res) = self.audio.play_instance(instance, false);
                if res != PlayResult::NotStarted {
                    self.cart_sounds.insert(cart.id);
                }
            }
        }
        let vehicle = self.riding.filter(|id| carts.iter().any(|c| c.id == *id));
        if vehicle != self.riding_cart_sounds
            && let Some(cart) = carts.iter().find(|c| Some(c.id) == vehicle)
        {
            let mut started = true;
            for underwater in [true, false] {
                let instance =
                    SoundInstance::riding_minecart(cart, underwater, random_source::create());
                let (_, res) = self.audio.play_instance(instance, false);
                if res == PlayResult::NotStarted {
                    started = false;
                }
            }
            if started {
                self.riding_cart_sounds = vehicle;
            }
        } else if vehicle.is_none() {
            self.riding_cart_sounds = None;
        }
        (vehicle, carts)
    }
    /// The rider's tick: the cart moves with the rider's input as its push.
    pub(super) fn tick_cart_riding(&mut self, keys: Keys) -> bool {
        let Some(i) = self.carts.iter().position(|c| Some(c.id) == self.riding) else {
            return false;
        };
        if keys.shift {
            self.dismount_cart(i);
            return true;
        }
        // ServerPlayer.getLastClientMoveIntent: getInputVector((left, 0, forward), 1, yRot).
        let left = f64::from(i8::from(keys.left) - i8::from(keys.right));
        let forward = f64::from(i8::from(keys.forward) - i8::from(keys.backward));
        let len = left.hypot(forward);
        let (left, forward) = if len > 1.0 {
            (left / len, forward / len)
        } else {
            (left, forward)
        };
        let r = self.yaw.to_radians();
        let (s, co) = (f64::from(r.sin()), f64::from(r.cos()));
        let intent = [left * co - forward * s, forward * co + left * s];
        let id = self.carts[i].id;
        let result =
            self.with_entity_held_out(id, |game| section_steps::tick_riding(game, id, i, intent));
        let Some((_, alive, cart_step, player_step)) = result else {
            return true;
        };
        section_steps::tell(self, id, cart_step);
        section_steps::tell_player(self, player_step);
        // AbstractMinecart.activateMinecart: a powered activator rail ejects
        // the rider, and getDismountLocationForPassenger places it beside the
        // cart instead of leaving it at the seat.
        if alive
            && self.riding.is_none()
            && let Some(i) = self.carts.iter().position(|cart| cart.id == id)
        {
            self.dismount_cart(i);
        }
        true
    }
    /// `getDismountLocationForPassenger`: beside the cart's motion direction,
    /// else on top.
    pub(super) fn dismount_cart(&mut self, i: usize) {
        let id = self.carts[i].id;
        if self.riding == Some(id) && !self.native_may_leave(id) {
            return;
        }
        self.riding = None;
        if self.dismount_in_portal(self.carts[i].position) {
            return;
        }
        let target = self.cart_player_dismount_target(i);
        let from = self.player.as_ref().map(|player| player.position);
        if let Some(p) = self.player.as_mut() {
            p.position = target;
            p.velocity = Vec3::ZERO;
        }
        section_steps::tell_player(self, from.map(|from| (from, target)));
    }
    /// Where the cart at `i` lets a player off.
    pub(super) fn cart_player_dismount_target(&self, i: usize) -> Vec3 {
        let c = &self.carts[i];
        let (bx, by, bz) = (
            c.position.x.floor() as i32,
            c.position.y.floor() as i32,
            c.position.z.floor() as i32,
        );
        let mut target = None;
        let face = cart_motion_face(c.yaw, c.flipped, self.minecart_improvements());
        'search: for dy in [0, 1, -1] {
            for (dx, dz) in dismount_offsets(face) {
                let (x, y, z) = (bx + dx, by + dy, bz + dz);
                let below = self.res.facts().get(self.world.block(x, y - 1, z));
                let here = self.res.facts().get(self.world.block(x, y, z));
                let head = self.res.facts().get(self.world.block(x, y + 1, z));
                if (below.full_collision || below.legacy_solid)
                    && !here.full_collision
                    && !head.full_collision
                    && here.fluid == Fluid::None
                {
                    target = Some(Vec3::new(
                        f64::from(x) + 0.5,
                        f64::from(y),
                        f64::from(z) + 0.5,
                    ));
                    break 'search;
                }
            }
        }
        let top = c.bounds().max_position().y;
        target.unwrap_or(Vec3::new(c.position.x, top, c.position.z))
    }
    /// `MinecartItem.useOn`: a cart on the clicked rail.
    pub(super) fn place_minecart(&mut self, hit: &Hit, stack: &ItemStack) -> bool {
        let Some(kind) = CartKind::from_id(&stack.item) else {
            return false;
        };
        let Some(rail) = self.rail(None, hit.pos[0], hit.pos[1], hit.pos[2]) else {
            return false;
        };
        let offset = if rail.shape.is_slope() { 0.5 } else { 0.0 };
        let at = Vec3::new(
            f64::from(hit.pos[0]) + 0.5,
            f64::from(hit.pos[1]) + 0.0625 + offset,
            f64::from(hit.pos[2]) + 0.5,
        );
        self.next_entity += 1;
        let cart = Minecart::new(self.next_entity, kind, at);
        self.world
            .modified
            .insert((hit.pos[0] >> 4, hit.pos[2] >> 4));
        self.carts.push(cart);
        let id = self.next_entity;
        self.apply_item_stack_config(super::commands::Target::Cart(id), kind.id(), stack, true);
        if self.serving.is_some() || self.options.dedicated.is_some() {
            let cart = self.carts.last().unwrap();
            self.native_vehicle_create(
                cart.kind.id(),
                [cart.position.x, cart.position.y, cart.position.z],
            );
        }
        // MinecartItem.useOn: ENTITY_PLACE at the rail, by the player, with the block under it.
        let below = self.world.block(hit.pos[0], hit.pos[1] - 1, hit.pos[2]);
        self.post_block_game_event(
            "minecraft:entity_place",
            hit.pos,
            self.sculk_actor(),
            Some(below),
        );
        if !self.creative {
            self.consume_held();
        }
        // ItemStack.useOn counts the use.
        self.award_stat("used", &stack.item, 1);
        self.swing_arm();
        true
    }
    pub(super) fn cart_hit(&self) -> Option<(usize, f64)> {
        let (eye, dir) = self.aim()?;
        let reach: f64 = self.entity_reach;
        let mut best: Option<(usize, f64)> = None;
        for (i, c) in self.carts.iter().enumerate() {
            if Some(c.id) == self.riding {
                continue;
            }
            let bb = c.bounds();
            let (lo, hi) = (bb.min_position(), bb.max_position());
            let (lo, hi) = ([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]);
            let (mut t0, mut t1) = (0.0f64, reach);
            let mut hit = true;
            for a in 0..3 {
                if dir[a].abs() < 1e-9 {
                    hit &= eye[a] >= lo[a] && eye[a] <= hi[a];
                    continue;
                }
                let (mut ta, mut tb) = ((lo[a] - eye[a]) / dir[a], (hi[a] - eye[a]) / dir[a]);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
            }
            if hit && t0 <= t1 && best.is_none_or(|b| t0 < b.1) {
                best = Some((i, t0));
            }
        }
        best
    }
    /// The slots of the cart `id`, for a command: `getChestVehicleItem` and
    /// `setChestVehicleItem` first unpack its loot table with no player, so
    /// no `player_generates_container_loot` fires later. While its menu is
    /// open the live slots are the placeholder copy (`open_cart_container`).
    pub(super) fn edit_cart_slots<R>(
        &mut self,
        id: u32,
        edit: impl FnOnce(&mut Vec<Option<ItemStack>>) -> R,
    ) -> Option<R> {
        let i = self.carts.iter().position(|c| c.id == id)?;
        self.unpack_loot(super::container_loot::LootTarget::Cart(id), None);
        match self
            .world
            .block_entities
            .get_mut(&(i32::MAX - 15, CART_KEY_Y, id as i32))
        {
            Some(BlockEntity::Chest(open)) => Some(edit(open)),
            _ => Some(self.carts.update(i, |c| edit(&mut c.items))),
        }
    }
    fn unpack_cart_loot(&mut self, i: usize) {
        self.unpack_loot(
            super::container_loot::LootTarget::Cart(self.carts[i].id),
            self.native_acting_player(),
        );
    }
    /// `interact`: ride, open or fuel the cart under the crosshair, or the one
    /// a guest's interact packet named. A TNT cart has no interact of its own
    /// (`Entity.interact` passes), so nothing here lights it.
    pub(super) fn interact_cart(&mut self, target: Option<InteractTarget>) -> bool {
        let Some((i, d)) = (match target {
            Some(t) => Self::named_entity(&self.carts, t.id, |c| c.id).map(|i| (i, 0.0)),
            None => self.cart_hit(),
        }) else {
            return false;
        };
        // Only a client's own pick is stopped by a block: the range check passed.
        if target.is_none() && !self.before_block(d) {
            return false;
        }
        let sneaking = self.keys.held(Action::Sneak);
        let held = self.held().cloned();
        match self.carts[i].kind {
            CartKind::Rideable => {
                let id = self.carts[i].id;
                // isVehicle: a mob already sits in it.
                let has_mob = !self.seated_passengers(id).is_empty();
                if sneaking || self.riding.is_some() || self.ridden_by_guest(id) || has_mob {
                    return false;
                }
                self.start_riding_target(
                    super::commands::Target::Player,
                    super::commands::Target::Cart(id),
                    false,
                );
            }
            // AbstractMinecartContainer.interact: no sneak check.
            CartKind::Chest | CartKind::Hopper => self.open_cart_container(i),
            CartKind::Furnace => {
                let fuel = held
                    .as_ref()
                    .is_some_and(|s| matches!(&*s.item, "minecraft:coal" | "minecraft:charcoal"));
                // Java int arithmetic: a mod can set any fuel.
                let fueled = self.carts.update(i, |c| {
                    if !fuel {
                        return false;
                    }
                    let Some(next) = furnace_fuel_after(c.fuel) else {
                        return false;
                    };
                    c.fuel = next;
                    if c.fuel > 0 {
                        let p = self.player.as_ref().map_or(c.position, |p| p.position);
                        c.push = Vec3::new(c.position.x - p.x, 0.0, c.position.z - p.z);
                    }
                    true
                });
                if fueled && !self.creative {
                    self.consume_held();
                }
            }
            // MinecartTNT has no interact of its own (`Entity.interact` passes).
            CartKind::Tnt => return false,
            // MinecartCommandBlock.interact: a game master gets SUCCESS (the
            // client opens the editor, `open_cart_command_block`), anyone else
            // PASSes to the held item. SUCCESS swings on the client only.
            CartKind::CommandBlock => return self.can_use_game_master_blocks(),
            // MinecartSpawner has no interact of its own (`Entity.interact` passes).
            CartKind::Spawner => return false,
        }
        self.swing_arm();
        true
    }
    /// `AbstractMinecartContainer.interact`: the menu opens over the cart's
    /// own slots, kept under a placeholder key until it closes
    /// (`store_cart_items`/`close_cart_container`).
    pub(super) fn open_cart_container(&mut self, i: usize) {
        self.unpack_cart_loot(i);
        let c = self.carts[i].clone();
        let key = (i32::MAX - 15, CART_KEY_Y, c.id as i32);
        // `startOpen` shares one container, it does not reset it: a stand-in
        // another player's menu is still editing over stays as it is.
        if !self.world.block_entities.contains_key(&key) {
            self.world
                .block_entities
                .insert(key, BlockEntity::Chest(c.items.clone()));
        }
        self.container = Some(key);
        self.set_screen(super::Screen::Inventory);
        self.capture(false);
        self.right = false;
        if c.kind == CartKind::Chest {
            // `MinecartChest.interact` (which overrides
            // `AbstractMinecartContainer.interact`): a successful open fires
            // CONTAINER_OPEN with the player and angers the piglins that see
            // it. `MinecartHopper` does not override it, so a hopper minecart
            // fires neither. Vanilla plays no open sound for a chest minecart.
            let at = [c.position.x, c.position.y, c.position.z];
            let ctx = self.player_context(None);
            self.post_game_event("minecraft:container_open", at, ctx);
            self.anger_piglins(true);
        }
    }
    /// Closing the cart's container open under `key` copies the items back.
    pub(super) fn close_cart_container(&mut self, key: (i32, i32, i32)) {
        if key.1 != CART_KEY_Y {
            return;
        }
        self.store_cart_items(key);
        let guest_viewers = self
            .lan
            .as_ref()
            .is_some_and(|host| Self::vehicle_has_other_viewers(None, &host.hub.guests, None, key));
        if !guest_viewers {
            self.world.block_entities.remove(&key);
        }
    }
    /// A chest or hopper minecart or a chest boat is about to be removed
    /// (`Containers.dropContents` / `AbstractChestBoat.remove`): what an open
    /// menu edited goes back to the vehicle, so it is what drops or travels
    /// with it, and the menu's stand-in leaves the level. A menu that is
    /// still open over it then has nothing left to take.
    pub(super) fn release_vehicle_container(&mut self, id: u32) {
        let key = (i32::MAX - 15, CART_KEY_Y, id as i32);
        self.store_cart_items(key);
        self.world.block_entities.remove(&key);
        if self.container == Some(key) {
            self.container = None;
        }
    }
    /// The menu of a chest or hopper minecart or a chest boat works on the
    /// vehicle's own items (`ContainerEntity`): the placeholder under `key`
    /// that the menu edits is copied back to the vehicle.
    pub(super) fn store_cart_items(&mut self, key: (i32, i32, i32)) {
        if key.1 != CART_KEY_Y {
            return;
        }
        let Some(BlockEntity::Chest(items)) = self.world.block_entities.get(&key) else {
            return;
        };
        let items = items.clone();
        if let Some(i) = self.carts.iter().position(|c| c.id as i32 == key.2) {
            self.carts.update(i, |c| c.items = items);
        } else if let Some(i) = self.boats.iter().position(|b| b.id as i32 == key.2) {
            self.boats.update(i, |b| {
                b.items = items;
            });
        }
    }
    /// The open cart's kind when a cart container is shown.
    pub(super) fn open_cart_kind(&self) -> Option<CartKind> {
        let key = self.container.filter(|k| k.1 == CART_KEY_Y)?;
        // A chest boat's chest shows like a chest minecart's.
        self.carts
            .iter()
            .find(|c| c.id as i32 == key.2)
            .map(|c| c.kind)
            .or_else(|| {
                self.boats
                    .iter()
                    .any(|b| b.chest && b.id as i32 == key.2)
                    .then_some(CartKind::Chest)
            })
    }
    /// An arrow hit cart `i` (`MinecartTNT.hurtServer`, then
    /// `VehicleEntity.hurtServer`): a burning arrow sets off a TNT minecart.
    /// `owner` is `AbstractArrow.getOwner`, the hit's `DamageSource.getEntity`;
    pub(super) fn arrow_hits_cart(
        &mut self,
        i: usize,
        damage: f32,
        arrow_velocity: Vec3,
        burning: bool,
        creative: bool,
        owner: Option<Who>,
        guest: Option<u32>,
    ) {
        if i >= self.carts.len() {
            return;
        }
        if burning && self.carts[i].kind == CartKind::Tnt {
            let mut c = self.carts[i].clone();
            if self.explode_cart(&mut c, arrow_velocity.length_squared()) {
                return;
            }
        }
        let hit = CartHit::projectile(owner, guest);
        self.hurt_cart(i, damage, creative, false, hit);
    }
    /// `Containers.dropContents(level, entity, container)` at a vehicle's
    /// position `at`: each slot (an empty one too) from a spot in its block,
    /// in pieces of 10 to 30 with the level random's scatter, no pickup
    /// delay. `AbstractChestBoat.remove` and `AbstractMinecartContainer.remove`
    /// spill this way for a destroying removal (kill or discard), and
    /// `chestVehicleDestroyed` again for the emptied container.
    pub(in crate::game) fn drop_vehicle_contents(
        &mut self,
        at: [f64; 3],
        items: &[Option<ItemStack>],
    ) {
        for stack in items {
            let count = stack.as_ref().map_or(0, |s| s.count);
            let pieces = horizon_world::level::removal::drop_item_stack_at(
                &mut self.world.random,
                at,
                count,
            );
            if let Some(stack) = stack {
                for piece in pieces {
                    let piece_stack = ItemStack {
                        count: piece.count,
                        ..stack.clone()
                    };
                    self.spawn_item(piece_stack, piece.pos, piece.motion, 0);
                }
            }
        }
    }
    /// Baked block-state faces with default block tints and terrain shade or diffuse lighting.
    pub(super) fn block_model_vertices(
        &self,
        state: BlockStateId,
        seed: i64,
        m: glam::Mat4,
        light: u32,
        diffuse: bool,
        out: &mut Vec<Vertex>,
    ) {
        // BlockTintSource.color(state) with no level: each tinted face's default colour.
        let grass = self.pack.item_renderer.grass_color(0.5, 1.0) as u32;
        let tables = &self.pack.context.tables;
        let tint = |index: i32| tables.default_tint(state, index, grass);
        self.tinted_block_model_vertices(state, seed, m, |_| light, diffuse, tint, out);
    }
    /// `block_model_vertices` with each face multiplied by `tint` of its
    /// tint index (RGB, `None` for untinted faces) and lit by `light` of
    /// each quad.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn tinted_block_model_vertices(
        &self,
        state: BlockStateId,
        seed: i64,
        m: glam::Mat4,
        light: impl Fn(&horizon_assets::bake::ModelQuad) -> u32,
        diffuse: bool,
        tint: impl Fn(i32) -> Option<u32>,
        out: &mut Vec<Vertex>,
    ) {
        let Ok(models) = self.pack.render.model_catalog.collect_seeded(state, seed) else {
            return;
        };
        let models = models.into_iter().map(|(_, model)| model);
        self.baked_model_vertices(models, m, light, diffuse, tint, out);
    }
    /// `tinted_block_model_vertices` for baked models given as they are (a
    /// state's, or a standalone model).
    pub(super) fn baked_model_vertices<'m>(
        &self,
        models: impl IntoIterator<Item = &'m horizon_client::models::catalog::BakedModel>,
        m: glam::Mat4,
        light: impl Fn(&horizon_assets::bake::ModelQuad) -> u32,
        diffuse: bool,
        tint: impl Fn(i32) -> Option<u32>,
        out: &mut Vec<Vertex>,
    ) {
        let faces = models.into_iter().flat_map(|model| {
            model.faces.iter().filter_map(|face| {
                let sprite = self
                    .pack
                    .layer_names
                    .get(&face.layer)
                    .and_then(|name| self.gui.atlas.sprites.get(name))?;
                Some((
                    PreparedModelFace::new(&face.quad, *sprite),
                    light(&face.quad),
                ))
            })
        });
        emit_model_faces(faces, m, diffuse, tint, out);
    }

    /// The block the cart shows: `None` for air (`AbstractMinecartRenderer`
    /// draws and stretches its culling box only for a non-air state).
    fn display_state(&self, c: &Minecart) -> Option<BlockStateId> {
        shown_unless_air(self.display_state_unfiltered(c), |s| {
            self.res.facts().is_air(s)
        })
    }
    fn display_state_unfiltered(&self, c: &Minecart) -> Option<BlockStateId> {
        let registry = &self.pack.assets.registry;
        let with = |name: &str, props: &[(&str, &str)]| {
            let mut s = registry.state(registry.default_state(name)?)?.clone();
            for (k, v) in props {
                s.properties.insert((*k).to_owned(), (*v).to_owned());
            }
            registry.id(&s)
        };
        // AbstractMinecart.getDisplayBlockState: a custom state wins.
        if let Some(state) = c
            .custom_display
            .as_ref()
            .and_then(|tag| horizon_client::entity_nbt::state_from_nbt(registry, tag))
        {
            return Some(state);
        }
        match c.kind {
            CartKind::Rideable => None,
            CartKind::Chest => with("minecraft:chest", &[("facing", "north")]),
            CartKind::Hopper => registry.default_state("minecraft:hopper"),
            CartKind::Furnace => with(
                "minecraft:furnace",
                &[
                    ("facing", "north"),
                    ("lit", if c.fuel > 0 { "true" } else { "false" }),
                ],
            ),
            CartKind::Tnt => registry.default_state("minecraft:tnt"),
            CartKind::CommandBlock => registry.default_state("minecraft:command_block"),
            // MinecartSpawner.getDefaultDisplayBlockState.
            CartKind::Spawner => registry.default_state("minecraft:spawner"),
        }
    }
    /// `AbstractMinecartRenderer` (old render): on the rail between the
    /// front and back points, pitched on slopes, with the display block.
    pub(in crate::game) fn cart_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        let a = alpha as f32;
        let Some(uv) = self
            .gui
            .atlas
            .sprites
            .get("entity/minecart/minecart")
            .map(|r| [r.u0, r.v0, r.u1, r.v1])
        else {
            return;
        };
        // Entity.shouldRenderAtSqrDistance: 64 times the bounding box's mean
        // edge, scaled by the view scale.
        let range = (WIDTH + HEIGHT + WIDTH) / 3.0 * 64.0 * self.view_scale();
        for c in &self.carts {
            let Some(model) = self.cart_models.models.get(&c.kind) else {
                continue;
            };
            let lerp = self.remote_minecart_lerp(c.id, a);
            let new_render = self.minecart_improvements() || lerp.is_some();
            let (x, y, z) = if let Some((p, _, _)) = lerp {
                (p[0], p[1], p[2])
            } else {
                (
                    c.previous.x + (c.position.x - c.previous.x) * alpha,
                    c.previous.y + (c.position.y - c.previous.y) * alpha,
                    c.previous.z + (c.position.z - c.previous.z) * alpha,
                )
            };
            // LevelExtractor.isEntityVisible: the cart the player rides always draws.
            let mut shown = None;
            if self.riding != Some(c.id) {
                let d = glam::DVec3::new(c.position.x, c.position.y, c.position.z) - camera;
                if d.length_squared() >= range * range {
                    continue;
                }
                // Looked up once, past the distance cut: the cull and the draw both use it.
                shown = Some(self.display_state(c));
                // EntityRenderer.shouldRender: the interpolated box grown by 0.5;
                // AbstractMinecartRenderer.getBoundingBoxForCulling stretches it
                // up by the display block's offset.
                if let Some(view) = &self.view_frustum {
                    let top = HEIGHT
                        + if shown.flatten().is_none() {
                            0.0
                        } else {
                            display_offset(c) * 0.75 / 16.0
                        };
                    let p = glam::DVec3::new(x, y, z) - camera;
                    let lo = p - glam::DVec3::new(WIDTH / 2.0 + 0.5, 0.5, WIDTH / 2.0 + 0.5);
                    let hi = p + glam::DVec3::new(WIDTH / 2.0 + 0.5, top + 0.5, WIDTH / 2.0 + 0.5);
                    if !view.intersects(lo.as_vec3(), hi.as_vec3()) {
                        continue;
                    }
                }
            }
            let mut yaw =
                c.previous_yaw + ((c.yaw - c.previous_yaw + 180.0).rem_euclid(360.0) - 180.0) * a;
            let mut pitch = 0.0f32;
            let mut offset = glam::DVec3::ZERO;
            if new_render {
                (yaw, pitch) = lerp.map_or((c.yaw, c.pitch), |(_, yaw, pitch)| (yaw, pitch));
            } else if let Some(on) = self.rail_pos(Some(c.id), x, y, z) {
                // getPosOffs(..., 0.3F): the float widened.
                let half = f64::from(0.3_f32);
                let front = self.rail_pos_offs(Some(c.id), x, y, z, half).unwrap_or(on);
                let back = self.rail_pos_offs(Some(c.id), x, y, z, -half).unwrap_or(on);
                offset = glam::DVec3::new(on[0] - x, (front[1] + back[1]) / 2.0 - y, on[2] - z);
                let d =
                    glam::DVec3::new(back[0] - front[0], back[1] - front[1], back[2] - front[2]);
                if d.length() != 0.0 {
                    let d = d.normalize();
                    yaw = d.z.atan2(d.x).to_degrees() as f32;
                    pitch = (d.y.atan() * 73.0) as f32;
                }
            }
            let render_position = Vec3::new(x + offset.x, y + offset.y, z + offset.z);
            let has_display = shown.as_ref().is_some_and(Option::is_some)
                || (shown.is_none() && self.display_state(c).is_some());
            let culling = self.entity_cull_decision_in_bounds(
                c,
                cart_render_bounds(render_position, display_offset(c), has_display),
            );
            if culling.hides_entity() {
                continue;
            }
            let entity_start = out.len();
            self.mark_motion(entity_start, Self::MOTION_OBJECT | u64::from(c.id));
            // The per-entity jitter (offsetSeed) that keeps stacked carts from z-fighting.
            let seed = i64::from(c.id).wrapping_mul(493_286_711);
            let seed = seed
                .wrapping_mul(seed)
                .wrapping_mul(4_392_167_121)
                .wrapping_add(seed.wrapping_mul(98_761));
            let jitter = |shift: i64| (((seed >> shift & 7) as f32 + 0.5) / 8.0 - 0.5) * 0.004;
            let p = glam::DVec3::new(x, y, z) + offset - camera;
            let jittered = p.as_vec3() + glam::Vec3::new(jitter(16), jitter(20), jitter(24));
            let mut base = if new_render {
                // AbstractMinecartRenderer.newRender translates the model
                // after rotating, so the seat follows the pitched cart.
                glam::Mat4::from_translation(jittered)
                    * glam::Mat4::from_rotation_y(yaw.to_radians())
                    * glam::Mat4::from_rotation_z((-pitch).to_radians())
                    * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.375, 0.0))
            } else {
                glam::Mat4::from_translation(jittered + glam::Vec3::new(0.0, 0.375, 0.0))
                    * glam::Mat4::from_rotation_y((180.0 - yaw).to_radians())
                    * glam::Mat4::from_rotation_z((-pitch).to_radians())
            };
            let hurt = c.hurt_time as f32 - a;
            if hurt > 0.0 {
                let damage = (c.damage - a).max(0.0);
                base *= glam::Mat4::from_rotation_x(
                    (hurt.sin() * hurt * damage / 10.0 * c.hurt_dir as f32).to_radians(),
                );
            }
            let l = self.world.light(
                c.position.x.floor() as i32,
                (c.position.y + 0.35).floor() as i32,
                c.position.z.floor() as i32,
            );
            let packed = (u32::from(l >> 4) * 16) | ((u32::from(l & 15) * 16) << 8);
            if let Some(state) = shown.unwrap_or_else(|| self.display_state(c)) {
                let display = base
                    * glam::Mat4::from_scale(glam::Vec3::splat(0.75))
                    * glam::Mat4::from_translation(glam::Vec3::new(
                        -0.5,
                        (display_offset(c) as f32 - 8.0) / 16.0,
                        0.5,
                    ))
                    * glam::Mat4::from_rotation_y(90f32.to_radians());
                if c.kind == CartKind::Chest {
                    self.special_block_model(state, display, l, out);
                } else {
                    // TNT minecarts flash white as the fuse runs out (TntMinecartRenderer).
                    let swell = if c.kind == CartKind::Tnt && c.fuse > -1 {
                        let fuse = c.fuse as f32 - a + 1.0;
                        if fuse < 10.0 {
                            let g = (1.0 - fuse / 10.0).clamp(0.0, 1.0);
                            1.0 + g * g * g * g * 0.3
                        } else {
                            1.0
                        }
                    } else {
                        1.0
                    };
                    let m = display
                        * glam::Mat4::from_translation(glam::Vec3::splat(0.5))
                        * glam::Mat4::from_scale(glam::Vec3::splat(swell))
                        * glam::Mat4::from_translation(glam::Vec3::splat(-0.5));
                    let flash = c.kind == CartKind::Tnt && c.fuse > -1 && (c.fuse / 5) % 2 == 0;
                    let start = out.len();
                    self.block_model_vertices(state, 0, m, packed, false, out);
                    if flash {
                        // submitWhiteSolidBlock: OverlayTexture.u(1.0) on the cart's own light.
                        for v in &mut out[start..] {
                            v.layer = 5;
                        }
                    }
                }
            }
            let m = base * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0));
            let poses = Poses::new(model);
            horizon_client::mobs::render::draw(model, m, &poses, uv, 0xFFFF_FFFF, packed, 0, out);
            self.finish_entity_occlusion(culling, c, out, entity_start);
        }
    }
    /// The model of a block drawn by its block-entity renderer, as a chest
    /// minecart shows its chest and a block display its special block.
    pub(super) fn special_block_model(
        &self,
        state: BlockStateId,
        m: glam::Mat4,
        light: u8,
        out: &mut Vec<Vertex>,
    ) {
        let lid = |_: [i32; 3]| 0.0;
        let scene = horizon_client::block_entity_render::Scene {
            camera: glam::DVec3::ZERO,
            orientation: glam::Quat::IDENTITY,
            anim: &|_| horizon_client::block_entity_render::BlockAnim::default(),
            texts: None,
            partial: 0.0,
            game_time: self.world.game_time,
            max_y: self.world.shape.max_y() - 1,
            world: &self.world,
            sprites: &self.gui.atlas.sprites,
            emissive: &self.gui.atlas.emissive,
            translucent_sprites: &self.gui.atlas.translucent,
            atlas_revision: self.gui.atlas.revision,
            nether: self.nether_lighting(),
            lid: &lid,
            sign: &|_| None,
            banner: &|_| Vec::new(),
            skull: &|_| None,
            glyph: &|c| self.gui.atlas.glyph_cell(c),
            glyphs: self.gui.atlas.font.epoch(),
            font: Some(&self.gui.atlas.font),
            dynamic_visible: None,
        };
        self.res
            .block_entity_render()
            .render_state(&scene, state.0, m, light, out);
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
            super::boats::put_ticks_frozen(map, self.ticks_frozen);
            let mut put = |k: &str, v: Tag| {
                map.insert(JavaString::from(k), v);
            };
            put("FlippedRotation", Tag::Byte(i8::from(self.flipped)));
            // AbstractMinecart saves firstTick as HasTicked. A cart never burns:
            // Entity.move sets -getFireImmuneTicks(), 0 for all but players.
            put("HasTicked", Tag::Byte(i8::from(self.first_tick)));
            put("Fire", Tag::Short(0));
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
        let int = |k: &str| {
            tag.get(k)
                .and_then(horizon_client_core::nbt_path::int_value)
        };
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
        c.ticks_frozen = super::boats::saved_ticks_frozen(tag);
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
                    horizon_client_core::nbt_path::int_value(other).map_or(default, |v| v as f32)
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
const DEFAULT_EXPLOSION_POWER: f32 = 4.0;
const DEFAULT_EXPLOSION_SPEED_FACTOR: f32 = 1.0;

/// `VehicleEntity.hurtServer`'s destroy test: a creative player, damage past
/// 40, or (`MinecartTNT.shouldSourceDestroy`) fire or an explosion hitting a
/// TNT cart.
fn cart_is_destroyed(creative: bool, damage: f32, ignites_tnt: bool) -> bool {
    creative || damage > 40.0 || ignites_tnt
}

/// `MinecartTNT.tick`'s collision detonation: a cart still moving at a tenth
/// of a block a tick or more when a block stops it blows up.
fn tnt_ignites_by_collision(horizontal_collision: bool, speed_sqr: f64) -> bool {
    horizontal_collision && speed_sqr >= 0.009_999_999_776_482_582
}

/// NewMinecartBehavior's `(float) (Math.atan2(y, x) * 180.0 / Math.PI)` before
/// the cast: HotSpot's atan2, multiplied then divided in Java's order.
fn java_degrees(y: f64, x: f64) -> f64 {
    horizon_core::jmath::atan2(y, x) * 180.0 / std::f64::consts::PI
}

#[cfg(test)]
#[path = "minecarts/minecart_tnt_tests.rs"]
mod minecart_tnt_tests;

/// `MinecartFurnace.addFuel`: Java int arithmetic, so a fuel a mod set near
/// `i32::MAX` wraps negative and takes more.
pub(super) fn furnace_fuel_after(fuel: i32) -> Option<i32> {
    let next = fuel.wrapping_add(3600);
    (next <= 32000).then_some(next)
}

/// `MinecartFurnace.getMaxSpeed`: half the behavior's speed, three quarters
/// in water.
fn furnace_speed(c: &Minecart, base: f64) -> f64 {
    if c.kind == CartKind::Furnace {
        base * if c.in_water { 0.75 } else { 0.5 }
    } else {
        base
    }
}

/// Local geometry with atlas coordinates resolved for both CPU and GPU consumers.
#[derive(Clone, Copy)]
pub(super) struct PreparedModelFace {
    positions: [[f32; 3]; 4],
    uv: [[f32; 2]; 4],
    pub(super) facing: horizon_assets::model::Direction,
    pub(super) tint_index: i32,
    light_emission: u8,
}

impl PreparedModelFace {
    pub(super) fn new(
        quad: &horizon_assets::bake::ModelQuad,
        sprite: horizon_client::hud::Rect,
    ) -> Self {
        Self {
            positions: quad.positions,
            uv: quad.uv.map(|uv| {
                [
                    sprite.u0 + uv[0] * (sprite.u1 - sprite.u0),
                    sprite.v0 + uv[1] * (sprite.v1 - sprite.v0),
                ]
            }),
            facing: quad.facing,
            tint_index: quad.tint_index,
            light_emission: quad.light_emission,
        }
    }

    pub(super) fn positions(&self) -> [[f32; 3]; 4] {
        self.positions
    }

    pub(in crate::game) fn gpu_vertex(
        &self,
        corner: usize,
    ) -> horizon_client::gfx::PreparedModelVertex {
        let normal = horizon_client::item_render::direction_vector(self.facing);
        horizon_client::gfx::PreparedModelVertex {
            pos: [
                self.positions[corner][0],
                self.positions[corner][1],
                self.positions[corner][2],
                1.0,
            ],
            uv: [self.uv[corner][0], self.uv[corner][1], 0.0, 0.0],
            normal: [normal.x, normal.y, normal.z, 0.0],
            flags: [
                u32::from(self.tint_index >= 0),
                u32::from(self.light_emission),
                0,
                0,
            ],
        }
    }
}

/// Shared transform, shading and light path for block-state and prepared standalone faces.
pub(super) fn emit_model_faces(
    faces: impl IntoIterator<Item = (PreparedModelFace, u32)>,
    m: glam::Mat4,
    diffuse: bool,
    tint: impl Fn(i32) -> Option<u32>,
    out: &mut Vec<Vertex>,
) {
    use horizon_assets::model::Direction as D;
    let mut normals = horizon_client::item_render::FaceNormals::new(m);
    for (quad, light) in faces {
        let shade: u32 = match quad.facing {
            D::Up => 255,
            D::Down => 127,
            D::North | D::South => 204,
            D::West | D::East => 153,
        };
        let mut color = if diffuse {
            u32::MAX
        } else {
            0xFF00_0000 | shade << 16 | shade << 8 | shade
        };
        if let Some(t) = tint(quad.tint_index) {
            color =
                horizon_client::hud_layout::multiply(color, horizon_client::mobs::render::abgr(t));
        }
        let normal = if diffuse {
            let n = normals
                .face(quad.facing)
                .to_array()
                .map(|v| (v.clamp(-1.0, 1.0) * 127.0) as i8);
            [n[0], n[1], n[2], 127]
        } else {
            [0; 4]
        };
        let light = if diffuse {
            let emission = u32::from(quad.light_emission) * 16;
            (light & 0xFF) | (light >> 8 & 0xFF).max(emission) << 8
        } else {
            light
        };
        for k in 0..4 {
            let uv = quad.uv[k];
            out.push(Vertex {
                normal,
                pos: m
                    .transform_point3(glam::Vec3::from_array(quad.positions[k]))
                    .to_array(),
                uv,
                layer: 0,
                color,
                light,
                glint_uv: [0.0, 0.0],
            });
        }
    }
}

/// Vanilla multiplies tint, alpha included, only when `tintIndex != -1`.
pub(in crate::game) fn scale_tint_alpha(
    vertices: &mut [Vertex],
    faces: &[PreparedModelFace],
    tint: u32,
) {
    let alpha = tint >> 24;
    if alpha == 255 {
        return;
    }
    let (quads, _) = vertices.as_chunks_mut::<4>();
    for (face, quad) in faces.iter().zip(quads) {
        if face.tint_index < 0 {
            continue;
        }
        for vertex in quad {
            let scaled = (vertex.color >> 24) * alpha / 255;
            vertex.color = vertex.color & 0x00FF_FFFF | scaled << 24;
        }
    }
}

#[cfg(test)]
#[path = "minecarts/tests.rs"]
mod tests;
horizon_client::impl_joined!(Minecart);
horizon_mod_runtime::impl_attachment_lifetime!(Minecart);

#[cfg(test)]
mod tnt_impact_tests {
    use super::{CartKind, Minecart};
    use crate::game::native_entity_refs::tests::with_game;
    use horizon_core::geometry::Vec3;

    /// Ticks a moving TNT cart until it stops or blows up; true when it
    /// blew up.
    fn runs_into_the_wall(rails: bool, at: Vec3, velocity: Vec3) -> bool {
        let mut result = false;
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let rail = registry.default_state("minecraft:rail").unwrap();
            let mut east_west = registry.state(rail).unwrap().clone();
            east_west
                .properties
                .insert("shape".into(), "east_west".into());
            let rail = registry.id(&east_west).unwrap();
            for x in 0..16 {
                for z in 0..16 {
                    game.world.set(x, 63, z, stone).unwrap();
                }
            }
            for z in 0..16 {
                game.world.set(9, 64, z, stone).unwrap();
                game.world.set(9, 65, z, stone).unwrap();
            }
            if rails {
                for x in 1..9 {
                    game.world.set(x, 64, 8, rail).unwrap();
                }
            }
            let mut cart = Minecart::new(1, CartKind::Tnt, at);
            cart.velocity = velocity;
            for _ in 0..40 {
                if !game.tick_cart(&mut cart, [0.0; 2], 0, &mut super::CartIndex::default()) {
                    result = true;
                    break;
                }
            }
        });
        result
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_powered_rail_pushes_a_resting_cart_away_from_soul_sand() {
        let mut resources = None;
        let mut game = crate::game::game_tests::tests::server_game(&mut resources);
        let registry = game.pack.assets.registry;
        let rail = registry.default_state("minecraft:powered_rail").unwrap();
        let mut state = registry.state(rail).unwrap().clone();
        state.properties.insert("shape".into(), "east_west".into());
        state.properties.insert("powered".into(), "true".into());
        let rail = registry.id(&state).unwrap();
        game.world.set(5, 64, 8, rail).unwrap();
        for (block, pushed) in [
            ("minecraft:soul_sand", true),
            ("minecraft:stone", true),
            ("minecraft:oak_slab", false),
        ] {
            let id = registry.default_state(block).unwrap();
            game.world.set(4, 64, 8, id).unwrap();
            let push = game.redstone_direction([5, 64, 8], super::Shape::EastWest);
            assert_eq!(push.x == 1.0, pushed, "{block}");
        }
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_tnt_cart_stopped_head_on_by_a_wall_does_not_blow_up() {
        // OldMinecartBehavior.moveAlongTrack: the move zeroes the blocked x
        // and the track's re-pointing scales what is left, so MinecartTNT
        // reads no speed at the wall.
        assert!(!runs_into_the_wall(
            true,
            Vec3::new(3.5, 64.0, 8.5),
            Vec3::new(0.4, 0.0, 0.0)
        ));
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_tnt_cart_off_its_rails_glancing_off_a_wall_blows_up() {
        // comeOffTrack keeps the delta the move resolved: the wall zeroes x,
        // z goes on at over a tenth of a block a tick.
        assert!(runs_into_the_wall(
            false,
            Vec3::new(8.3, 64.0, 4.5),
            Vec3::new(0.4, 0.0, 0.4)
        ));
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_cart_on_a_slope_presses_a_plate_at_its_temporary_position() {
        // OldMinecartBehavior.moveAlongTrack places the cart at the rail's top
        // height, moves it horizontally (Entity.move records the step) and settles
        // it on the slope; the tick's applyEffectsFromBlocks still sweeps the step.
        let mut resources = None;
        let mut game = crate::game::game_tests::tests::server_game(&mut resources);
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let plate = registry
            .default_state("minecraft:oak_pressure_plate")
            .unwrap();
        let rail = registry.default_state("minecraft:rail").unwrap();
        let mut slope = registry.state(rail).unwrap().clone();
        slope
            .properties
            .insert("shape".into(), "ascending_east".into());
        let rail = registry.id(&slope).unwrap();
        for x in 0..16 {
            for z in 0..16 {
                game.world.set(x, 63, z, stone).unwrap();
            }
        }
        game.world.set(5, 64, 8, rail).unwrap();
        game.world.set(6, 64, 8, stone).unwrap();
        game.world.set(6, 65, 8, plate).unwrap();
        let mut cart = Minecart::new(1, CartKind::Rideable, Vec3::new(5.5, 64.5, 8.5));
        cart.velocity = Vec3::new(0.4, 0.0, 0.0);
        game.carts.push(cart.clone());
        assert!(game.tick_cart(&mut cart, [0.0; 2], 0, &mut super::CartIndex::default()));
        let powered = registry
            .state(game.world.block(6, 65, 8))
            .unwrap()
            .properties
            .get("powered")
            .cloned();
        assert_eq!(
            powered.as_deref(),
            Some("true"),
            "{:?}",
            registry.state(game.world.block(6, 65, 8))
        );
    }

    /// A cart blocked by a fence on a slope sits below the plate on the fence,
    /// but `AbstractMinecart.move` sweeps its blocks while it is still at the
    /// rail's top height, so the plate clocks on and off.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_cart_resting_against_a_fence_keeps_toggling_the_plate_above_it() {
        let mut resources = None;
        let mut game = crate::game::game_tests::tests::server_game(&mut resources);
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let fence = registry.default_state("minecraft:oak_fence").unwrap();
        let plate = registry
            .default_state("minecraft:oak_pressure_plate")
            .unwrap();
        let rail = registry.default_state("minecraft:rail").unwrap();
        let mut slope = registry.state(rail).unwrap().clone();
        slope
            .properties
            .insert("shape".into(), "ascending_east".into());
        let slope = registry.id(&slope).unwrap();
        for x in 0..16 {
            for z in 0..16 {
                game.world.set(x, 62, z, stone).unwrap();
            }
        }
        game.world.set(0, 64, 8, fence).unwrap();
        game.world.set(0, 65, 8, plate).unwrap();
        game.world.set(1, 63, 8, stone).unwrap();
        game.world.set(1, 64, 8, slope).unwrap();
        game.world.set(2, 64, 8, stone).unwrap();
        game.world.set(2, 65, 8, rail).unwrap();
        game.carts.push(Minecart::new(
            1,
            CartKind::Rideable,
            Vec3::new(1.9, 64.9, 8.5),
        ));
        // Scheduled ticks and carts only run in lit, surrounded chunks near a
        // player.
        game.settings.simulation_distance = 4;
        game.settings.render_distance = 12;
        game.player = Some(
            horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 70.0, 12.5))
                .unwrap(),
        );
        for cx in -2..=2 {
            for cz in -2..=2 {
                if (cx, cz) != (0, 0) {
                    let mut chunk = horizon_client::chunk::Chunk::empty(cx, cz);
                    chunk.lit = true;
                    game.world
                        .chunks
                        .insert((cx, cz), std::sync::Arc::new(chunk));
                    game.block_ticks
                        .add_empty_chunk(horizon_core::position::ChunkPos::new(cx, cz))
                        .unwrap();
                }
            }
        }
        let mut center = (**game.world.chunks.get(&(0, 0)).unwrap()).clone();
        center.lit = true;
        game.world
            .chunks
            .insert((0, 0), std::sync::Arc::new(center));
        game.update_chunk_tickets();
        let mut edges = 0;
        let mut last = false;
        for _ in 0..200 {
            game.world.game_time += 1;
            game.tick_carts_now();
            game.tick_blocks().unwrap();
            let on = registry
                .state(game.world.block(0, 65, 8))
                .unwrap()
                .properties
                .get("powered")
                .is_some_and(|v| v == "true");
            if on != last {
                edges += 1;
                last = on;
            }
        }
        assert!(edges >= 4, "the plate changed {edges} times in 200 ticks");
    }
}

#[cfg(test)]
mod cart_push_tests {
    use super::{CartKind, Minecart};
    use horizon_client::{items::ItemStack, save::PendingContainerLoot};
    use horizon_core::geometry::Vec3;

    /// Two chest carts pushed apart (`OldMinecartBehavior.pushAndPickupEntities`):
    /// the push runs on copies of the ticked cart, and the one still holding
    /// its mineshaft roll keeps the roll, so it is saved as its loot table.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_pushed_chest_cart_keeps_its_pending_loot() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            for x in 4..13 {
                for z in 4..13 {
                    game.world.set(x, 63, z, stone).unwrap();
                }
            }
            let items = vec![Some(ItemStack::new("minecraft:rail", 12)); 27];
            let loot = PendingContainerLoot {
                table: horizon_core::nbt::Tag::String(
                    "minecraft:chests/abandoned_mineshaft".into(),
                ),
                seed: Some(horizon_core::nbt::Tag::Long(42)),
            };
            let mut rolled = Minecart::new(1, CartKind::Chest, Vec3::new(8.3, 64.0, 8.5));
            rolled.items = items;
            rolled.loot = Some(loot.clone());
            game.carts.push(rolled);
            // Facing along X (yaw 0) and 0.4 apart on X: the old behaviour pushes.
            game.carts
                .push(Minecart::new(2, CartKind::Chest, Vec3::new(8.7, 64.0, 8.5)));
            for _ in 0..5 {
                game.tick_carts();
            }
            let cart = |id| game.carts.iter().find(|c| c.id == id).unwrap();
            assert!(
                cart(2).position.x - cart(1).position.x > 0.4,
                "the carts were pushed apart: {} {}",
                cart(1).position.x,
                cart(2).position.x
            );
            assert_eq!(cart(1).loot.as_ref(), Some(&loot));
            assert_eq!(cart(2).loot, None);
        });
    }

    /// `OldMinecartBehavior.pushAndPickupEntities` for carts: a moving rideable
    /// cart only pushes what `EntitySelector.pushableBy(cart)` allows, by its
    /// own team's collision rule and the target's; a still cart keeps vanilla's
    /// unfiltered minecart push.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_moving_carts_team_rules_which_carts_it_pushes() {
        use crate::game::{Game, commands::Target};
        use horizon_client::scoreboard::CollisionRule;

        /// Puts a cart on a team of its own, as `/team join` would.
        fn on_a_team(game: &mut Game<'_>, id: u32, rule: CollisionRule) {
            let name = game.holder_name(Target::Cart(id)).expect("a cart's UUID");
            let team = format!("cart-{id}");
            game.score.board.add_team(&team).collision_rule = rule;
            game.score.board.add_to_team(&name, &team);
        }
        /// Pushes with cart `pusher` moving at `velocity`, and returns the
        /// target's velocity afterwards.
        fn push_apart(game: &mut Game<'_>, pusher: usize, target: usize, velocity: Vec3) -> Vec3 {
            game.carts.update(target, |cart| cart.velocity = Vec3::ZERO);
            game.carts.update(pusher, |cart| cart.velocity = velocity);
            let mut index = super::CartIndex::default();
            game.push_from_cart(pusher, &mut index);
            game.carts[target].velocity
        }

        let mut resources = None;
        let mut game = crate::game::game_tests::tests::server_game(&mut resources);
        // Cart 2 moves east, into cart 1's box; yaw 0 faces east, along the push.
        game.carts.push(Minecart::new(
            1,
            CartKind::Rideable,
            Vec3::new(8.5, 64.0, 8.5),
        ));
        game.carts.push(Minecart::new(
            2,
            CartKind::Rideable,
            Vec3::new(8.1, 64.0, 8.5),
        ));
        let east = Vec3::new(0.2, 0.0, 0.0);
        // No teams: the moving cart pushes the still one.
        assert_ne!(
            push_apart(&mut game, 1, 0, east),
            Vec3::ZERO,
            "the moving cart pushes the still one"
        );
        // The target's `never` rule holds the push back.
        on_a_team(&mut game, 1, CollisionRule::Never);
        assert_eq!(
            push_apart(&mut game, 1, 0, east),
            Vec3::ZERO,
            "the target's collisionRule never keeps it off"
        );
        // The moving cart's own rule does too.
        game.score.board.remove_team("cart-1");
        on_a_team(&mut game, 2, CollisionRule::Never);
        assert_eq!(
            push_apart(&mut game, 1, 0, east),
            Vec3::ZERO,
            "the moving cart's collisionRule never keeps it off"
        );
        // Allies with `pushOwnTeam` do not push each other.
        game.score.board.remove_team("cart-2");
        on_a_team(&mut game, 1, CollisionRule::PushOwnTeam);
        let name = game.holder_name(Target::Cart(2)).expect("a cart's UUID");
        game.score.board.add_to_team(&name, "cart-1");
        assert_eq!(
            push_apart(&mut game, 1, 0, east),
            Vec3::ZERO,
            "pushOwnTeam keeps allies apart"
        );
        // A still cart is vanilla's else branch: no team filter at all.
        assert_ne!(
            push_apart(&mut game, 1, 0, Vec3::ZERO),
            Vec3::ZERO,
            "a still cart pushes the minecart in its box whatever the teams"
        );
    }

    /// `EntitySelector.pushableBy(cart)` for mobs: a moving rideable cart on a
    /// `never` team neither boards nor pushes the cow in its path.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_moving_carts_team_keeps_it_from_boarding_a_cow() {
        use crate::game::commands::Target;
        use horizon_client::{mobs::Mob, scoreboard::CollisionRule};

        let mut resources = None;
        let mut game = crate::game::game_tests::tests::server_game(&mut resources);
        let kind = game.res.mob_types().get("minecraft:cow").cloned().unwrap();
        let mut cart = Minecart::new(2, CartKind::Rideable, Vec3::new(8.5, 64.0, 8.5));
        cart.velocity = Vec3::new(0.2, 0.0, 0.0);
        game.carts.push(cart);
        let at = Vec3::new(8.5, 64.0, 8.5);
        game.mobs.push(Mob::new(7, kind, at, 0.0));
        let mut index = super::CartIndex::default();
        game.push_from_cart(0, &mut index);
        assert_eq!(
            game.mobs[0].extra.mount,
            Some(2),
            "the cow boards the empty moving cart"
        );
        game.mobs[0].extra.mount = None;
        game.mobs[0].velocity = Vec3::ZERO;
        // The cart's own `never` rule: no boarding and no push.
        let name = game.holder_name(Target::Cart(2)).expect("a cart's UUID");
        game.score.board.add_team("cart-2").collision_rule = CollisionRule::Never;
        game.score.board.add_to_team(&name, "cart-2");
        game.carts
            .update(0, |cart| cart.velocity = Vec3::new(0.2, 0.0, 0.0));
        let mut index = super::CartIndex::default();
        game.push_from_cart(0, &mut index);
        assert_eq!(
            game.mobs[0].extra.mount, None,
            "the cart's collisionRule never keeps the cow off"
        );
        assert_eq!(
            game.mobs[0].velocity,
            Vec3::ZERO,
            "and the cow is not pushed either"
        );
    }
}

#[cfg(test)]
mod container_drop_tests {
    use super::super::boats::Boat;
    use super::{CartHit, CartKind, Minecart};
    use horizon_client::items::ItemStack;
    use horizon_core::geometry::Vec3;
    use horizon_core::random::{LegacyRandom, RandomSource};
    use std::sync::Arc;

    /// Advances `r` past the draws a spill takes from the level random, as
    /// vanilla's `Containers.dropContents` takes them: for each slot
    /// (`Containers.dropItemStack`) its spot, then for each pile the split
    /// size and the triangle motion. Every count here is at most 10, so a
    /// slot with items is a single pile.
    fn vanilla_spill_draws(r: &mut LegacyRandom, counts: &[u32]) {
        for &count in counts {
            for _ in 0..3 {
                r.next_double();
            }
            if count > 0 {
                let _ = r.next_int_bound(21);
                for _ in 0..3 {
                    r.triangle(0.0, 0.114_850_001_711_398_36);
                }
            }
        }
    }

    /// A chest boat or chest minecart broken with entity_drops on spills its
    /// slots in `remove` and then the emptied container again in
    /// `chestVehicleDestroyed`; the hull item takes the item entity's own
    /// random. The level random must end where vanilla's does.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_broken_chest_vehicle_draws_the_level_random_like_vanilla() {
        let mut slots = vec![None; 27];
        slots[0] = Some(ItemStack::new("minecraft:diamond", 5));
        slots[4] = Some(ItemStack::new("minecraft:dirt", 8));
        let mut counts = vec![0; 27];
        counts[0] = 5;
        counts[4] = 8;
        for boat in [true, false] {
            crate::game::native_entity_refs::tests::with_game(|game| {
                assert!(
                    game.ui
                        .rules
                        .bool_at(horizon_client::gamerules::vanilla::ENTITY_DROPS)
                );
                let at = Vec3::new(8.5, 64.0, 8.5);
                if boat {
                    let mut b = Boat::new(1, Arc::from("oak"), at, 0.0);
                    b.chest = true;
                    b.items = slots.clone();
                    game.boats.push(b);
                } else {
                    let mut c = Minecart::new(1, CartKind::Chest, at);
                    c.items = slots.clone();
                    game.carts.push(c);
                }
                game.world.random.set_seed(1234);
                if boat {
                    assert_eq!(game.hurt_boat_result(0, 100.0, false), Some(true));
                    assert!(game.boats.is_empty());
                } else {
                    assert!(game.hurt_cart_result(0, 100.0, false, false, CartHit::default()));
                    assert!(game.carts.is_empty());
                }
                let mut expected = LegacyRandom::new(1234);
                vanilla_spill_draws(&mut expected, &counts);
                vanilla_spill_draws(&mut expected, &[0; 27]);
                assert_eq!(
                    game.world.random.next_long(),
                    expected.next_long(),
                    "boat: {boat}"
                );
                // The two piles and the hull item.
                assert_eq!(game.dropped.len(), 3, "boat: {boat}");
            });
        }
    }
}

#[cfg(test)]
mod rejected_passenger_tests {
    use super::*;
    use crate::game::{commands::Target, native_entity_refs::tests::with_game};
    use horizon_core::nbt::Tag;

    fn pig(uuid: i32, children: Vec<Tag>) -> Tag {
        Tag::compound([
            ("id", Tag::String("minecraft:pig".into())),
            ("UUID", Tag::IntArray(vec![0, 2175, 0, uuid])),
            (
                "Pos",
                Tag::List(vec![
                    Tag::Double(8.0f64.to_bits()),
                    Tag::Double(64.0f64.to_bits()),
                    Tag::Double(8.0f64.to_bits()),
                ]),
            ),
            ("Passengers", Tag::List(children)),
        ])
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn rejected_cart_passenger_occupies_capacity_physics_pickup_and_interaction() {
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let cart = 5000;
            game.carts.push(Minecart::new(
                cart,
                CartKind::Rideable,
                Vec3::new(8.0, 64.0, 8.0),
            ));
            game.load_mob(pig(1, vec![]), None).unwrap();
            let ghost = game
                .load_mob(pig(1, vec![pig(2, vec![])]), Some(cart))
                .unwrap();
            assert!(game.persist.off_level.contains_key(&ghost));
            let candidate = game.load_mob(pig(3, vec![]), None).unwrap();
            assert!(game.cart_is_vehicle(cart));
            let mut copy = game.carts[0].clone();
            copy.velocity = Vec3::new(1.0, 0.0, 0.0);
            assert_eq!(
                game.natural_slowdown(&mut copy, game.cart_is_vehicle(cart), 0.96)
                    .x,
                0.997
            );
            assert!(!game.start_riding_target(Target::Mob(candidate), Target::Cart(cart), false));
            game.carts
                .update(0, |c| c.velocity = Vec3::new(0.2, 0.0, 0.0));
            game.pickup_from_cart(0, &mut CartIndex::default());
            assert_eq!(game.mob_object(candidate).unwrap().extra.mount, None);
            assert!(!game.interact_cart(Some(InteractTarget {
                id: cart as i32,
                location: [0.0; 3]
            })));
            assert_eq!(game.riding, None);
            let mut camel = pig(4, vec![]);
            if let Tag::Compound(c) = &mut camel {
                c.insert("id".into(), Tag::String("minecraft:camel".into()));
            }
            let camel = game.load_mob(camel, None).unwrap();
            game.load_mob(pig(1, vec![]), Some(camel)).unwrap();
            game.load_mob(pig(1, vec![]), Some(camel)).unwrap();
            game.load_mob(pig(1, vec![]), Some(camel)).unwrap();
            assert_eq!(game.seated_passengers(camel).len(), 3);
            assert!(!game.start_riding_target(Target::Mob(candidate), Target::Mob(camel), false));
            let mut active = game.carts[0].clone();
            active.position = Vec3::new(40.0, 70.0, 40.0);
            active.yaw = 90.0;
            active.flipped = true;
            game.activate_cart(&mut active, true);
            let rider = game.mob_object(ghost).unwrap();
            assert_eq!(rider.extra.mount, None);
            assert_eq!(rider.position, Vec3::new(40.0, 70.0 + HEIGHT, 40.0));
            assert_eq!(rider.previous, rider.position);
            let leaf = game.mob_passenger_ids(ghost)[0];
            assert_eq!(game.mob_object(leaf).unwrap().position.x, 40.0);
            assert!(game.mob_object(leaf).unwrap().position.y >= 70.0);
            assert_eq!(game.carts[0].position, Vec3::new(8.0, 64.0, 8.0));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn cart_destruction_ejects_rejected_rider_and_snaps_frozen_descendant() {
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            game.carts.push(Minecart::new(
                5000,
                CartKind::Rideable,
                Vec3::new(8.0, 70.0, 8.0),
            ));
            game.load_mob(pig(1, vec![]), None).unwrap();
            let ghost = game
                .load_mob(pig(1, vec![pig(2, vec![])]), Some(5000))
                .unwrap();
            let leaf = game.mob_passenger_ids(ghost)[0];
            assert!(game.mobs_below_off_level().contains(&leaf));
            game.eject_cart_mobs(0);
            let rider = game.mob_object(ghost).unwrap();
            assert_eq!(rider.extra.mount, None);
            assert!(!rider.extra.passenger);
            assert_eq!(rider.position.y, 70.0);
            assert_eq!(rider.position, rider.previous);
            assert_eq!(game.mob_object(leaf).unwrap().extra.mount, Some(ghost));
            assert!(game.mob_object(leaf).unwrap().position.y >= 70.0);
        });
    }
}

#[cfg(test)]
mod hook_mutation_regressions {
    use super::*;
    use horizon_client::mobs::Mob;
    use std::sync::{Arc, Mutex};

    struct MutateRider {
        discard: bool,
        heard: Arc<Mutex<Vec<(u32, bool)>>>,
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for MutateRider {
        fn entity_mount(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            self.heard
                .lock()
                .unwrap()
                .push((event.entity, event.mounting));
            if !event.mounting && event.entity == 72 {
                if self.discard {
                    assert!(host.entity(72).unwrap().discard(host));
                } else {
                    let later = host.entity(73).unwrap();
                    let replacement = host.entity(74).unwrap();
                    assert!(later.start_riding(host, replacement, true));
                }
            }
            mod_api::Flow::Continue
        }
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn activator_preserves_a_later_passengers_callback_reparenting() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "mutate_rider",
            Box::new(MutateRider {
                discard: false,
                heard: heard.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.carts
                    .push(Minecart::new(71, CartKind::Rideable, Vec3::ZERO));
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                for id in [72, 73, 74] {
                    let mut mob = Mob::new(id, kind.clone(), Vec3::ZERO, 0.0);
                    mob.extra.mount = (id != 74).then_some(71);
                    mob.extra.vehicle_order = i64::from(id);
                    game.mobs.push(mob);
                }
                let mut active = game.carts[0].clone();
                game.activate_cart(&mut active, true);
                assert_eq!(game.mob_object(73).unwrap().extra.mount, Some(74));
                assert_eq!(
                    heard
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|&&(id, mounting)| id == 73 && !mounting)
                        .count(),
                    1,
                    "only the callback's deliberate reparenting dismounts this rider"
                );
            });
        });
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn activator_does_not_snap_a_rider_discarded_by_its_callback() {
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "mutate_rider",
            Box::new(MutateRider {
                discard: true,
                heard: Arc::default(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.carts
                    .push(Minecart::new(71, CartKind::Rideable, Vec3::ZERO));
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                let original = Vec3::new(2.0, 60.0, 3.0);
                let mut rider = Mob::new(72, kind.clone(), original, 0.0);
                rider.extra.mount = Some(71);
                game.mobs.push(rider);
                let mut child = Mob::new(73, kind, original, 0.0);
                child.extra.mount = Some(72);
                game.mobs.push(child);
                let mut active = game.carts[0].clone();
                active.position = Vec3::new(40.0, 70.0, 40.0);
                game.activate_cart(&mut active, true);
                assert!(game.mob_object(72).unwrap().removed);
                assert_eq!(game.mob_object(72).unwrap().position, original);
                assert_eq!(game.mob_object(73).unwrap().position, original);
            });
        });
    }
    struct DiscardDamagedVehicle(Arc<Mutex<usize>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for DiscardDamagedVehicle {
        fn vehicle_damage(
            &self,
            host: &mut dyn mod_api::ServerHost,
            _: &mut mod_api::VehicleDamageEvent<'_>,
        ) -> mod_api::Flow {
            assert!(host.entity(71).unwrap().discard(host));
            mod_api::Flow::Continue
        }
        fn vehicle_destroy(
            &self,
            _: &mut dyn mod_api::ServerHost,
            _: &mut mod_api::VehicleDestroyEvent<'_>,
        ) {
            *self.0.lock().unwrap() += 1;
        }
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_damage_callback_discard_does_not_also_destroy_or_drop_the_vehicle() {
        for cart in [false, true] {
            let destroyed = Arc::new(Mutex::new(0));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "discard_damaged",
                Box::new(DiscardDamagedVehicle(destroyed.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                super::super::native_entity_refs::tests::with_game(|game| {
                    game.options.dedicated = Some(Vec::new());
                    game.ui
                        .rules
                        .set("entity_drops", horizon_client::gamerules::Value::Bool(true));
                    if cart {
                        game.carts
                            .push(Minecart::new(71, CartKind::Rideable, Vec3::ZERO));
                        game.hurt_cart(
                            0,
                            5.0,
                            false,
                            false,
                            crate::game::minecarts::CartHit::default(),
                        );
                    } else {
                        game.boats.push(super::super::boats::Boat::new(
                            71,
                            "oak".into(),
                            Vec3::ZERO,
                            0.0,
                        ));
                        game.hurt_boat(0, 5.0, false);
                    }
                    assert_eq!(*destroyed.lock().unwrap(), 0);
                    assert!(
                        game.dropped.is_empty(),
                        "discard must not create the vehicle item"
                    );
                    game.native_removals_flush();
                    assert_eq!(*destroyed.lock().unwrap(), 0);
                    assert!(game.dropped.is_empty());
                    assert!(game.entity(71).is_none());
                });
            });
        }
    }
}

#[cfg(test)]
mod tnt_cart_power_tests {
    use super::*;

    #[test]
    fn tnt_cart_power_round_trips_live_state_and_clears_stale_defaults() {
        use horizon_core::nbt::{Tag, parse_snbt};
        let raw = parse_snbt("{Pos:[1d,200d,1d],explosion_power:200f,explosion_speed_factor:-2d}")
            .unwrap()
            .0;
        let mut cart = Minecart::from_nbt(&raw, 1, CartKind::Tnt).unwrap();
        assert_eq!(cart.explosion_power, 128.0);
        assert_eq!(cart.explosion_speed_factor, 0.0);
        let saved = cart.to_nbt([0; 4], Some(&raw));
        assert_eq!(
            saved.get("explosion_power"),
            Some(&Tag::Float(128.0f32.to_bits()))
        );
        assert_eq!(
            saved.get("explosion_speed_factor"),
            Some(&Tag::Float(0.0f32.to_bits()))
        );
        cart.explosion_power = DEFAULT_EXPLOSION_POWER;
        cart.explosion_speed_factor = DEFAULT_EXPLOSION_SPEED_FACTOR;
        let saved = cart.to_nbt([0; 4], Some(&raw));
        assert!(saved.get("explosion_power").is_none());
        assert!(saved.get("explosion_speed_factor").is_none());
        let restored = Minecart::from_nbt(&saved, 2, CartKind::Tnt).unwrap();
        assert_eq!(restored.explosion_power, DEFAULT_EXPLOSION_POWER);
        assert_eq!(
            restored.explosion_speed_factor,
            DEFAULT_EXPLOSION_SPEED_FACTOR
        );
    }
}

#[cfg(test)]
mod cart_cost_bench {
    use super::{CartKind, Minecart};
    use crate::game::game_tests::tests::server_game;
    use horizon_core::geometry::Vec3;

    /// Times `tick_carts` with many parked carts and some moving ones:
    /// `CARTS_PARKED` (default 600) carts each on its own rail piece, and
    /// `CARTS_MOVING` (default 100) on powered tracks. Prints the ms per tick.
    #[test]
    #[ignore = "a timing, needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn tick_carts_with_many_parked_carts() {
        let n = |key: &str, default: usize| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        let (parked, moving) = (n("CARTS_PARKED", 600), n("CARTS_MOVING", 100));
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.settings.simulation_distance = 6;
        game.settings.render_distance = 12;
        game.player = Some(
            horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 80.0, 12.5))
                .unwrap(),
        );
        for cx in -3..=3 {
            for cz in -3..=3 {
                if (cx, cz) != (0, 0) {
                    let mut chunk = horizon_client::chunk::Chunk::empty(cx, cz);
                    chunk.lit = true;
                    game.world
                        .chunks
                        .insert((cx, cz), std::sync::Arc::new(chunk));
                    game.block_ticks
                        .add_empty_chunk(horizon_core::position::ChunkPos::new(cx, cz))
                        .unwrap();
                }
            }
        }
        game.update_chunk_tickets();
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let rail = registry.default_state("minecraft:rail").unwrap();
        let powered = registry.default_state("minecraft:powered_rail").unwrap();
        let with = |base: horizon_world::registry::BlockStateId, shape: &str, on: bool| {
            let mut s = registry.state(base).unwrap().clone();
            s.properties.insert("shape".into(), shape.into());
            if on {
                s.properties.insert("powered".into(), "true".into());
            }
            registry.id(&s).unwrap()
        };
        let (rail_ns, rail_ew) = (
            with(rail, "north_south", false),
            with(powered, "east_west", true),
        );
        for x in -30..46 {
            for z in -30..46 {
                game.world.set(x, 63, z, stone).unwrap();
                game.world.set(x, 69, z, stone).unwrap();
            }
        }
        let mut id = 1;
        'park: for x in (-28..44).step_by(2) {
            for z in (-28..44).step_by(2) {
                if id as usize > parked {
                    break 'park;
                }
                game.world.set(x, 64, z, rail_ns).unwrap();
                let at = Vec3::new(f64::from(x) + 0.5, 64.0, f64::from(z) + 0.5);
                game.carts.push(Minecart::new(id, CartKind::Rideable, at));
                id += 1;
            }
        }
        for lane in 0..10 {
            let z = -28 + lane * 6;
            for x in -30..46 {
                game.world.set(x, 70, z, rail_ew).unwrap();
            }
            for k in 0..moving.div_ceil(10) {
                if id as usize > parked + moving {
                    break;
                }
                let at = Vec3::new(-29.5 + f64::from(k as i32) * 3.0, 70.0, f64::from(z) + 0.5);
                let mut c = Minecart::new(id, CartKind::Rideable, at);
                c.velocity = Vec3::new(0.3, 0.0, 0.0);
                game.carts.push(c);
                id += 1;
            }
        }
        let chicken = game
            .res
            .mob_types()
            .get("minecraft:chicken")
            .unwrap()
            .clone();
        for k in 0..n("MOBS", 3000) {
            let (x, z) = (
                -30.0 + (k % 76) as f64 + 0.5,
                -30.0 + ((k / 76) % 76) as f64 + 0.5,
            );
            game.mobs.push(horizon_client::mobs::Mob::new(
                1000 + k as u32,
                chicken.clone(),
                Vec3::new(x, 75.0, z),
                0.0,
            ));
        }
        let total = game.carts.len();
        let (warm, ticks) = (5, n("TICKS", 40));
        let mut spent = std::time::Duration::ZERO;
        for t in 0..warm + ticks {
            game.world.game_time += 1;
            // Mobs move every tick on a live server: the list is written.
            if !game.mobs.is_empty() {
                let _ = &mut game.mobs[0];
            }
            let start = std::time::Instant::now();
            game.tick_carts();
            if t >= warm {
                spent += start.elapsed();
            }
        }
        eprintln!(
            "tick_carts: {total} carts ({parked} parked, {moving} moving): {:.3} ms/tick",
            spent.as_secs_f64() * 1e3 / ticks as f64
        );
    }
}

#[cfg(test)]
#[path = "minecarts/minecart_riders_tests.rs"]
mod minecart_riders_tests;
