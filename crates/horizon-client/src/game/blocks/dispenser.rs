//! Dispenser behaviours: `DispenserBlock.getDispenseMethod(..).dispense` with
//! every `DispenseItemBehavior` registration of vanilla that Horizon has the
//! entities for. A behaviour returns what is left in the slot and the level
//! event it plays (`DefaultDispenseItemBehavior.dispense` then plays that
//! event and the smoke animation, event 2000).
use super::level_sounds::LevelSound;
use super::{Game, item_abilities::ability};
use horizon_client::{
    Result,
    collisions::SceneCollisions,
    containers::BlockEntity,
    items::{ItemStack, Items},
    mobs::{Mob, Species, equip::equippable, sulfur_cube},
    particles::{ParticleOptions, ParticleType},
    projectiles::{Projectile, ProjectileKind},
};
use mod_api::SoundSource;
// Its old home, for mixin code that names it here.
use horizon_core::{
    geometry::{Aabb, Direction, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::{
    fluid::FluidKind,
    level::{PlaceContext, PlaceOutcome},
    registry::BlockStateId,
};
use std::sync::Arc;

pub(super) type Pos = (i32, i32, i32);

/// The sound event of a plain click, a failed dispense and a launched projectile.
pub(super) const CLICK: i32 = 1000;
pub(super) const FAIL: i32 = 1001;
const LAUNCH: i32 = 1002;

fn step(d: Direction) -> [i32; 3] {
    match d {
        Direction::Down => [0, -1, 0],
        Direction::Up => [0, 1, 0],
        Direction::North => [0, 0, -1],
        Direction::South => [0, 0, 1],
        Direction::West => [-1, 0, 0],
        Direction::East => [1, 0, 0],
    }
}
pub(super) fn offset(p: Pos, d: Direction) -> Pos {
    let s = step(d);
    (p.0 + s[0], p.1 + s[1], p.2 + s[2])
}
/// `Direction.get3DDataValue`.
fn data_value(d: Direction) -> i32 {
    match d {
        Direction::Down => 0,
        Direction::Up => 1,
        Direction::North => 2,
        Direction::South => 3,
        Direction::West => 4,
        Direction::East => 5,
    }
}
/// `Direction.toYRot` (vertical directions give 270 through `data2d = -1`).
fn y_rot(d: Direction) -> f32 {
    match d {
        Direction::South => 0.0,
        Direction::West => 90.0,
        Direction::North => 180.0,
        Direction::East => 270.0,
        _ => 270.0,
    }
}
/// `DirectionalPlaceContext.getRotation`: `get2DDataValue() * 90`, so a
/// vertical direction (`data2d = -1`) gives -90 where `toYRot` gives 270: the
/// same horizontal facing, but the raw value a mod's placement hook reads.
fn place_rotation(d: Direction) -> f32 {
    match d {
        Direction::Up | Direction::Down => -90.0,
        _ => y_rot(d),
    }
}
fn block_box(p: Pos) -> Aabb {
    Aabb::new(
        Vec3::new(f64::from(p.0), f64::from(p.1), f64::from(p.2)),
        Vec3::new(f64::from(p.0 + 1), f64::from(p.1 + 1), f64::from(p.2 + 1)),
    )
}
/// `EntitySectionStorage.forEachAccessibleNonEmptySection`: x ascending, then
/// the packed key (`SectionPos.asLong`: z as 22 unsigned bits, then y as 20);
/// within a section the entities go in the order they joined, which ids follow.
fn section_order(at: Vec3) -> (i32, u32, u32) {
    let s = |v: f64| (v.floor() as i32) >> 4;
    let (x, y, z) = (s(at.x), s(at.y), s(at.z));
    (x, z as u32 & 0x3F_FFFF, y as u32 & 0xF_FFFF)
}
fn overlaps(a: Aabb, b: Aabb) -> bool {
    let (a0, a1, b0, b1) = (
        a.min_position(),
        a.max_position(),
        b.min_position(),
        b.max_position(),
    );
    a0.x < b1.x && b0.x < a1.x && a0.y < b1.y && b0.y < a1.y && a0.z < b1.z && b0.z < a1.z
}
/// `ItemStack.shrink(1)`; the slot is empty at zero.
fn shrink(mut stack: ItemStack) -> Option<ItemStack> {
    stack.count = stack.count.saturating_sub(1);
    (stack.count > 0).then_some(stack)
}
/// A dispenser's answer: the slot's new content and the level event played.
pub(super) type Dispensed = (Option<ItemStack>, i32);

/// None keeps vanilla's behaviour; an explicit empty remainder consumes the stack.
fn mod_dispensed(answer: Option<mod_api::Dispensed>, items: &Items) -> Option<Dispensed> {
    use super::api_values::TryIntoEngine;
    answer.and_then(|(stack, event)| Some((stack.try_into_engine(items).ok()?, event)))
}

thread_local! {
    /// `OptionalDispenseItemBehavior.success` of the registered behaviours
    /// that leave it unset on some paths (brush, TNT, honeycomb, chest).
    /// Vanilla keeps one behaviour instance per item for the whole run, so
    /// there the flag carries over from that item's previous dispense.
    static SUCCESS: std::cell::RefCell<std::collections::HashMap<&'static str, bool>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}
fn set_success(behaviour: &'static str, success: bool) {
    SUCCESS.with(|s| s.borrow_mut().insert(behaviour, success));
}
/// The level event `OptionalDispenseItemBehavior.playSound` plays for the flag.
fn optional_event(behaviour: &'static str) -> i32 {
    if SUCCESS.with(|s| s.borrow().get(behaviour).copied().unwrap_or(true)) {
        CLICK
    } else {
        FAIL
    }
}

/// Entities `getEntitiesOfClass(.., AABB(pos))` can find.
#[derive(Clone, Copy)]
enum Target {
    Player,
    /// A server player, by connection: the integrated owner and every guest.
    Guest(u32),
    Mob(usize),
    Stand(usize),
}

/// `EquipmentSlot`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Slot {
    MainHand,
    OffHand,
    Feet,
    Legs,
    Chest,
    Head,
    Body,
    Saddle,
}
impl Slot {
    pub(super) fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "mainhand" => Self::MainHand,
            "offhand" => Self::OffHand,
            "feet" => Self::Feet,
            "legs" => Self::Legs,
            "chest" => Self::Chest,
            "head" => Self::Head,
            "body" => Self::Body,
            "saddle" => Self::Saddle,
            _ => return None,
        })
    }
    /// The player's armour array index (head, chest, legs, feet).
    fn armor_index(self) -> Option<usize> {
        match self {
            Self::Head => Some(0),
            Self::Chest => Some(1),
            Self::Legs => Some(2),
            Self::Feet => Some(3),
            _ => None,
        }
    }
    /// `ArmorStand` equipment index (mainhand, offhand, feet, legs, chest, head).
    fn stand_index(self) -> Option<usize> {
        match self {
            Self::MainHand => Some(0),
            Self::OffHand => Some(1),
            Self::Feet => Some(2),
            Self::Legs => Some(3),
            Self::Chest => Some(4),
            Self::Head => Some(5),
            _ => None,
        }
    }
}

fn equine(s: Species) -> bool {
    matches!(
        s,
        Species::Horse
            | Species::Donkey
            | Species::Mule
            | Species::SkeletonHorse
            | Species::ZombieHorse
            | Species::Camel
            | Species::CamelHusk
            | Species::Llama
            | Species::TraderLlama
    )
}
/// Mobs with species-specific body equipment.
pub(super) fn body_equipment_species(s: Species) -> bool {
    equine(s)
        || matches!(s, Species::Wolf | Species::HappyGhast | Species::SulfurCube)
        || horizon_client::mobs::nautilus::is_nautilus(s)
}
/// What a mob holds in a slot.
pub(super) fn mob_slot_item(m: &Mob, slot: Slot) -> Option<Arc<str>> {
    let x = &m.extra;
    match slot {
        Slot::MainHand | Slot::OffHand | Slot::Feet | Slot::Legs | Slot::Chest | Slot::Head => {
            horizon_client::mobs::finalize::item(m, slot as usize)
        }
        Slot::Body => x.body_stack().map(|s| s.item.clone()),
        Slot::Saddle => m.saddle_item().map(|s| s.item),
    }
}
/// `LivingEntity.canUseSlot` and its overrides.
fn mob_can_use_slot(m: &Mob, slot: Slot) -> bool {
    let s = m.species();
    let grown = m.alive() && !m.is_baby();
    match s {
        Species::Pig | Species::Strider if slot == Slot::Saddle => grown,
        Species::Horse
        | Species::SkeletonHorse
        | Species::ZombieHorse
        | Species::Llama
        | Species::TraderLlama => true,
        _ if equine(s) && slot == Slot::Saddle => grown && m.extra.tame,
        _ if horizon_client::mobs::nautilus::is_nautilus(s)
            && matches!(slot, Slot::Saddle | Slot::Body) =>
        {
            grown && m.extra.tame
        }
        Species::SulfurCube | Species::HappyGhast if slot == Slot::Body => grown,
        _ => true,
    }
}
/// `Mob.canDispenserEquipIntoSlot` and its overrides.
fn mob_can_dispenser_equip(m: &Mob, slot: Slot) -> bool {
    let s = m.species();
    let loot = m.extra.pick_up_loot;
    match s {
        Species::Pig | Species::Strider => slot == Slot::Saddle || loot,
        _ if equine(s) => matches!(slot, Slot::Body | Slot::Saddle) && m.extra.tame || loot,
        _ if horizon_client::mobs::nautilus::is_nautilus(s) => {
            matches!(slot, Slot::Body | Slot::Saddle) || loot
        }
        Species::SulfurCube | Species::HappyGhast => slot == Slot::Body,
        Species::Dolphin | Species::Panda | Species::Fox => slot == Slot::MainHand && loot,
        Species::Allay => false,
        _ => loot,
    }
}
/// `getEquipSound`: the saddle overrides of pigs, striders, horses, camels and nautili.
pub(super) fn equip_sound(m: &Mob, slot: Slot, default: Option<String>) -> Option<String> {
    if slot != Slot::Saddle {
        return default;
    }
    let s = m.species();
    Some(match s {
        Species::Pig => "minecraft:entity.pig.saddle".into(),
        Species::Strider => "minecraft:entity.strider.saddle".into(),
        Species::Camel => "minecraft:entity.camel.saddle".into(),
        Species::CamelHusk => "minecraft:entity.camel_husk.saddle".into(),
        _ if equine(s) => "minecraft:entity.horse.saddle".into(),
        _ if horizon_client::mobs::nautilus::is_nautilus(s) => {
            if m.fluids.eye_in_water {
                "minecraft:item.nautilus_saddle_underwater_equip".into()
            } else {
                "minecraft:item.nautilus_saddle_equip".into()
            }
        }
        _ => return default,
    })
}
pub(super) fn mob_set_slot(m: &mut Mob, slot: Slot, stack: ItemStack) {
    m.set_equipment_stack(slot as usize, Some(stack));
}

impl Game<'_> {
    fn prop(&self, state: BlockStateId, key: &str) -> Option<String> {
        self.pack
            .assets
            .registry
            .state(state)?
            .properties
            .get(key)
            .cloned()
    }
    /// The same block with other properties (`withProperty`).
    pub(super) fn with_props(
        &self,
        state: BlockStateId,
        props: &[(&str, &str)],
    ) -> Option<BlockStateId> {
        let mut s = self.pack.assets.registry.state(state)?.clone();
        for (k, v) in props {
            s.properties.insert((*k).to_owned(), (*v).to_owned());
        }
        self.pack.assets.registry.resolve(&s).ok()
    }
    fn block_at(&self, p: Pos) -> BlockStateId {
        self.world.block(p.0, p.1, p.2)
    }
    fn dispenser_block_name(&self, p: Pos) -> std::sync::Arc<str> {
        self.res.facts().get(self.block_at(p)).name.clone()
    }
    fn in_block_tag(&self, tag: &str, block: &str) -> bool {
        self.res
            .items()
            .block_tags
            .get(tag)
            .is_some_and(|s| s.contains(block))
    }
    fn water_at(&self, p: Pos) -> bool {
        matches!(
            self.res.behaviours().fluid_kind(self.block_at(p)),
            FluidKind::Water | FluidKind::FlowingWater
        )
    }
    fn center(p: Pos) -> [f64; 3] {
        [
            f64::from(p.0) + 0.5,
            f64::from(p.1) + 0.5,
            f64::from(p.2) + 0.5,
        ]
    }
    pub(super) fn pos_of(p: Pos) -> BlockPos {
        BlockPos::new(p.0, p.1, p.2)
    }

    // ---- level events (LevelEventHandler) ----

    /// The dispenser level events: 1000 click, 1001 fail, 1002 launch, 1004
    /// firework shoot, 1018 blaze shoot and 2000 smoke shot out of the front.
    pub(super) fn dispenser_event(&mut self, id: i32, pos: Pos, data: i32) {
        super::dispense_fixture::record(|| format!("L {id} {},{},{} {data}", pos.0, pos.1, pos.2));
        let c = Self::center(pos);
        match id {
            1000 => self
                .audio
                .play("minecraft:block.dispenser.dispense", Some(c), 1.0, 1.0),
            1001 => self
                .audio
                .play("minecraft:block.dispenser.fail", Some(c), 1.0, 1.2),
            1002 => self
                .audio
                .play("minecraft:block.dispenser.launch", Some(c), 1.0, 1.2),
            1004 => self
                .audio
                .play("minecraft:entity.firework_rocket.shoot", Some(c), 1.0, 1.2),
            // LevelEventHandler runs on the client: its pitch draws come from
            // the client level's random, never the server stream.
            1018 => {
                let r = self.particles.random();
                let pitch = (r.next_float() - r.next_float()) * 0.2 + 1.0;
                self.audio
                    .play("minecraft:entity.blaze.shoot", Some(c), 2.0, pitch);
            }
            1051 => {
                let pitch = 0.4 / (self.particles.random().next_float() * 0.4 + 0.8);
                self.audio
                    .play("minecraft:entity.wind_charge.throw", Some(c), 0.5, pitch);
            }
            1049 => self
                .audio
                .play("minecraft:block.crafter.craft", Some(c), 1.0, 1.0),
            1050 => self
                .audio
                .play("minecraft:block.crafter.fail", Some(c), 1.0, 1.0),
            // LevelEventHandler.shootParticles: smoke for dispensers, white
            // smoke out of a crafter's front.
            2000 | 2010 => {
                let particle = if id == 2010 {
                    ParticleType::WhiteSmoke
                } else {
                    ParticleType::Smoke
                };
                let s = match data {
                    0 => [0, -1, 0],
                    1 => [0, 1, 0],
                    2 => [0, 0, -1],
                    3 => [0, 0, 1],
                    4 => [-1, 0, 0],
                    _ => [1, 0, 0],
                };
                for _ in 0..10 {
                    let r = self.particles.random();
                    let pow = r.next_double() * 0.2 + 0.01;
                    let (sx, sy, sz) = (f64::from(s[0]), f64::from(s[1]), f64::from(s[2]));
                    let p = [
                        f64::from(pos.0)
                            + sx * 0.6
                            + 0.5
                            + sx * 0.01
                            + (r.next_double() - 0.5) * sz * 0.5,
                        f64::from(pos.1)
                            + sy * 0.6
                            + 0.5
                            + sy * 0.01
                            + (r.next_double() - 0.5) * sy * 0.5,
                        f64::from(pos.2)
                            + sz * 0.6
                            + 0.5
                            + sz * 0.01
                            + (r.next_double() - 0.5) * sx * 0.5,
                    ];
                    let v = [
                        sx * pow + r.next_gaussian() * 0.01,
                        sy * pow + r.next_gaussian() * 0.01,
                        sz * pow + r.next_gaussian() * 0.01,
                    ];
                    self.particles.add(&ParticleOptions::simple(particle), p, v);
                }
            }
            _ => {}
        }
    }

    // ---- DefaultDispenseItemBehavior ----

    /// `DispenserBlock.getDispensePosition(source, scale, offset)`; vanilla adds
    /// `scale * step + offset` first, then adds that sum to the block centre.
    fn dispense_position(pos: Pos, facing: Direction, scale: f64, offset: [f64; 3]) -> Vec3 {
        let s = step(facing);
        let c = Self::center(pos);
        Vec3::new(
            c[0] + (scale * f64::from(s[0]) + offset[0]),
            c[1] + (scale * f64::from(s[1]) + offset[1]),
            c[2] + (scale * f64::from(s[2]) + offset[2]),
        )
    }
    /// `DefaultDispenseItemBehavior.spawnItem` at `getDispensePosition`.
    pub(super) fn spawn_dispensed(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
        accuracy: i32,
    ) {
        let s = step(facing);
        let p = Self::dispense_position(pos, facing, 0.7, [0.0; 3]);
        let mut at = [p.x, p.y, p.z];
        at[1] -= if s[1] != 0 { 0.125 } else { 0.156_25 };
        let r = &mut self.world.random;
        let pow = r.next_double() * 0.1 + 0.2;
        let spread = 0.017_227_5 * f64::from(accuracy);
        let v = [
            r.triangle(f64::from(s[0]) * pow, spread),
            r.triangle(0.2, spread),
            r.triangle(f64::from(s[2]) * pow, spread),
        ];
        self.spawn_item_thrown(stack, at, v);
    }
    /// `DefaultDispenseItemBehavior.execute`: drop one item.
    fn default_execute(
        &mut self,
        pos: Pos,
        facing: Direction,
        mut stack: ItemStack,
    ) -> Option<ItemStack> {
        let one = ItemStack {
            count: 1,
            ..stack.clone()
        };
        stack.count -= 1;
        self.spawn_dispensed(pos, facing, one, 6);
        (stack.count > 0).then_some(stack)
    }
    /// Drop one item as DefaultDispenseItemBehavior does. For a dispenser hook:
    /// return this remainder in its answer with `CLICK`; the caller plays the
    /// answer's event and smoke once. A client, another dimension or an unloaded
    /// chunk keeps the input stack unchanged.
    #[allow(dead_code)]
    pub(super) fn dispense_default(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        facing: Direction,
        stack: ItemStack,
    ) -> Option<ItemStack> {
        if !(self.serving.is_some() || self.options.dedicated.is_some())
            || dimension != self.dim.id()
            || !self.world.chunks.contains_key(&(pos[0] >> 4, pos[2] >> 4))
        {
            return Some(stack);
        }
        if stack.count == 0 {
            return None;
        }
        self.default_execute((pos[0], pos[1], pos[2]), facing, stack)
    }
    /// `new DefaultDispenseItemBehavior().dispense(..)` including its events.
    pub(super) fn dispense_default_in_level(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Option<ItemStack> {
        let left = self.default_execute(pos, facing, stack);
        self.dispenser_event(CLICK, pos, 0);
        self.dispenser_event(2000, pos, data_value(facing));
        left
    }
    /// `DefaultDispenseItemBehavior.consumeWithRemainder`: the remainder goes
    /// into the dispenser (`insertItem`) or is dropped.
    pub(super) fn consume_with_remainder(
        &mut self,
        pos: Pos,
        facing: Direction,
        mut stack: ItemStack,
        remainder: ItemStack,
    ) -> Option<ItemStack> {
        stack.count -= 1;
        if stack.count == 0 {
            return Some(remainder);
        }
        let mut rest = remainder;
        if let Some(entity) = self.world.block_entities.get_mut(&pos) {
            let max = self
                .res
                .items()
                .get(&rest.item)
                .map_or(64, |i| i.max_stack)
                .max(1);
            for cell in entity.slots_mut() {
                match cell {
                    None => {
                        let n = rest.count.min(max);
                        *cell = Some(ItemStack {
                            count: n,
                            ..rest.clone()
                        });
                        rest.count -= n;
                    }
                    Some(e) if e.same_item_same_components(&rest) => {
                        let n = rest.count.min(max.saturating_sub(e.count));
                        e.count += n;
                        rest.count -= n;
                    }
                    Some(_) => {}
                }
                if rest.count == 0 {
                    break;
                }
            }
        }
        if rest.count > 0 {
            self.spawn_dispensed(pos, facing, rest, 6);
            self.dispenser_event(CLICK, pos, 0);
            self.dispenser_event(2000, pos, data_value(facing));
        }
        Some(stack)
    }
    /// `ItemStack.hurtAndBreak(amount)` without an entity.
    fn hurt_and_break(&mut self, stack: ItemStack, amount: u32) -> Option<ItemStack> {
        self.damage_stack(&stack, amount.min(i32::MAX as u32) as i32, false)
            .0
    }

    // ---- the registry ----

    /// `DispenserBlock.getDispenseMethod(..).dispense(source, stack)`: what the
    /// slot holds afterwards.
    pub(super) fn dispense_item(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Option<ItemStack>> {
        let mut answer = None;
        if self.serving.is_some() || self.options.dedicated.is_some() {
            // The block the dispenser faces has the first say (`dispense_method`), then the
            // item's own part (`dispense_item`); the first answer wins.
            let front = self.block_at(offset(pos, facing));
            self.native_dispense_method(
                self.dim.id(),
                [pos.0, pos.1, pos.2],
                facing,
                front,
                &stack,
                &mut answer,
            );
            if answer.is_none() {
                self.native_dispense_item(
                    self.dim.id(),
                    [pos.0, pos.1, pos.2],
                    facing,
                    &stack,
                    &mut answer,
                );
            }
        }
        if let Some((left, event)) = mod_dispensed(answer, self.items()) {
            if event == FAIL {
                self.native_block_failed_dispense(
                    self.dim.id(),
                    [pos.0, pos.1, pos.2],
                    &stack.item,
                );
            }
            // DefaultDispenseItemBehavior.dispense: playSound, then playAnimation;
            // a NOOP behaviour (event 0) plays neither.
            if event != 0 {
                self.dispenser_event(event, pos, 0);
                self.dispenser_event(2000, pos, data_value(facing));
            }
            return Ok(left);
        }
        let item = stack.item.to_string();
        // A stack able to `shears_harvest` is dispensed as shears are, whatever its item.
        let shears = self.stack_shears(&stack, ability::SHEARS_HARVEST);
        let (left, event) = match item.as_str() {
            _ if shears => self.dispense_shears(pos, facing, stack)?,

            "minecraft:arrow"
            | "minecraft:tipped_arrow"
            | "minecraft:spectral_arrow"
            | "minecraft:egg"
            | "minecraft:blue_egg"
            | "minecraft:brown_egg"
            | "minecraft:snowball"
            | "minecraft:experience_bottle"
            | "minecraft:splash_potion"
            | "minecraft:lingering_potion"
            | "minecraft:firework_rocket"
            | "minecraft:fire_charge"
            | "minecraft:wind_charge" => self.dispense_projectile(pos, facing, stack),
            "minecraft:armor_stand" => self.dispense_armor_stand(pos, facing, stack),
            "minecraft:chest" => self.dispense_chest(pos, facing, stack),
            "minecraft:lava_bucket"
            | "minecraft:water_bucket"
            | "minecraft:powder_snow_bucket"
            | "minecraft:salmon_bucket"
            | "minecraft:cod_bucket"
            | "minecraft:pufferfish_bucket"
            | "minecraft:tropical_fish_bucket"
            | "minecraft:axolotl_bucket"
            | "minecraft:tadpole_bucket"
            | "minecraft:sulfur_cube_bucket" => self.dispense_filled_bucket(pos, facing, stack)?,
            "minecraft:bucket" => self.dispense_empty_bucket(pos, facing, stack)?,
            "minecraft:flint_and_steel" => self.dispense_flint_and_steel(pos, facing, stack)?,
            "minecraft:bone_meal" => self.dispense_bone_meal(pos, facing, stack)?,
            "minecraft:tnt" => self.dispense_tnt(pos, facing, stack),
            "minecraft:wither_skeleton_skull" => self.dispense_wither_skull(pos, facing, stack)?,
            "minecraft:carved_pumpkin" => self.dispense_carved_pumpkin(pos, facing, stack)?,
            "minecraft:glass_bottle" => self.dispense_glass_bottle(pos, facing, stack)?,
            "minecraft:glowstone" => self.dispense_glowstone(pos, facing, stack)?,
            "minecraft:brush" => self.dispense_brush(pos, facing, stack),
            "minecraft:honeycomb" => self.dispense_honeycomb(pos, facing, stack)?,
            "minecraft:potion" => self.dispense_potion(pos, facing, stack)?,
            _ if item.ends_with("shulker_box") => self.dispense_shulker_box(pos, facing, stack)?,
            _ if self.dispenses_as_filled_bucket(&item) => {
                self.dispense_filled_bucket(pos, facing, stack)?
            }
            _ if super::boats::any_boat_wood(&item).is_some() => {
                self.dispense_boat(pos, facing, stack)
            }
            _ if super::minecarts::CartKind::from_id(&item).is_some() => {
                self.dispense_minecart(pos, facing, stack)
            }
            _ => self.dispense_default_method(pos, facing, stack)?,
        };
        if event == FAIL {
            self.native_block_failed_dispense(self.dim.id(), [pos.0, pos.1, pos.2], &item);
        }
        self.dispenser_event(event, pos, 0);
        self.dispenser_event(2000, pos, data_value(facing));
        Ok(left)
    }
    /// `DispenserBlock.getDefaultDispenseMethod`: equipment, spawn eggs, or a drop.
    fn dispense_default_method(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        // `ItemStack.has(DataComponents.EQUIPPABLE)`: the stack's own, a removed one included.
        if equippable(self.items(), &stack).is_some() {
            let mut stack = stack;
            if self.dispense_equipment(pos, facing, &mut stack) {
                return Ok((Some(stack).filter(|s| s.count > 0), CLICK));
            }
            return Ok((self.default_execute(pos, facing, stack), CLICK));
        }
        let swallowable = self
            .res
            .items()
            .item_tags
            .get("minecraft:sulfur_cube_swallowable")
            .is_some_and(|t| t.contains(&*stack.item));
        if swallowable {
            let front = offset(pos, facing);
            if self.sulfur_cube_swallow(front, &stack) {
                return Ok((shrink(stack), CLICK));
            }
            return Ok((self.default_execute(pos, facing, stack), CLICK));
        }
        // DispenserBlock.getDefaultDispenseMethod: an egg without ENTITY_DATA drops as an item.
        if stack.item.ends_with("_spawn_egg")
            && stack
                .data()
                .is_none_or(|data| data.other("!minecraft:entity_data").is_none())
        {
            return self.dispense_spawn_egg(pos, facing, stack);
        }
        Ok((self.default_execute(pos, facing, stack), CLICK))
    }

    // ---- entities ----

    /// Entities whose box overlaps the block at `p`, in `getEntitiesOfClass`
    /// order ([`section_order`]): players are entities of the level like any
    /// other, so they sort with the mobs and stands.
    fn targets_at(&self, p: Pos) -> Vec<Target> {
        let block = block_box(p);
        let key = |at: Vec3, id: u32| (section_order(at), id);
        let mut found: Vec<(_, Target)> = self
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.removed && overlaps(m.bounds(), block))
            .map(|(i, m)| (key(m.position, m.id), Target::Mob(i)))
            .collect();
        found.extend(
            self.armor_stands
                .iter()
                .enumerate()
                .filter(|(_, s)| overlaps(s.bounds(), block))
                .map(|(i, s)| (key(s.position, s.id), Target::Stand(i))),
        );
        // `ServerLevel.getEntities`: every player placed in this level, the
        // integrated owner included (a server has no player of its own), its
        // client loaded or not (it joins the level at placeNewPlayer/respawn).
        if let Some(host) = &self.lan {
            found.extend(
                (host.hub.guests.iter())
                    .filter(|g| host.hub.here(g) && !g.dead && !g.spectator())
                    .filter(|g| overlaps(g.bounds(), block))
                    .map(|g| {
                        (
                            key(
                                Vec3::new(g.position[0], g.position[1], g.position[2]),
                                g.entity_id as u32,
                            ),
                            Target::Guest(g.conn),
                        )
                    }),
            );
        }
        if let Some(v) = self.player_view()
            && v.alive
            && !self.spectator()
            && overlaps(v.bounds, block)
        {
            // The local player joins before the chunks around it load their
            // entities (prepareLevels loads only saved tickets): the lowest id,
            // 0 as `own_entity_id` has it.
            found.push((key(v.position, 0), Target::Player));
        }
        found.sort_by_key(|(k, _)| *k);
        found.into_iter().map(|(_, t)| t).collect()
    }
    /// `EntityType.getYOffset(level, pos, false, box)`: [`Self::drop_offset_from`] without
    /// `movedUp`.
    pub(super) fn drop_offset(&self, width: f64, height: f64, p: Pos) -> f64 {
        self.drop_offset_from(width, height, p, false)
    }
    /// `EntityType.getYOffset(level, pos, movedUp, box)`: the entity box set on
    /// top of the block (`setPos(x + 0.5, y + 1, z + 0.5)`) falls one block
    /// against the collision shapes overlapping that block, or, `moved_up`, two
    /// blocks against those of the block and the one under it.
    pub(super) fn drop_offset_from(&self, width: f64, height: f64, p: Pos, moved_up: bool) -> f64 {
        use horizon_world::{
            block_collision_scan::BlockCollisionScan, collision_context::CollisionContext,
        };
        let h = width / 2.0;
        let (x, z) = (f64::from(p.0) + 0.5, f64::from(p.2) + 0.5);
        let y = f64::from(p.1) + 1.0;
        let entity = Aabb::new(
            Vec3::new(x - h, y, z - h),
            Vec3::new(x + h, y + height, z + h),
        );
        let cell = Aabb::new(
            Vec3::new(
                f64::from(p.0),
                f64::from(p.1) - if moved_up { 1.0 } else { 0.0 },
                f64::from(p.2),
            ),
            Vec3::new(
                f64::from(p.0) + 1.0,
                f64::from(p.1) + 1.0,
                f64::from(p.2) + 1.0,
            ),
        );
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        let mut amount = if moved_up { -2.0 } else { -1.0 };
        // Level.getCollisions(null, aabb): CollisionContext.empty().
        if let Ok(scan) = BlockCollisionScan::new(
            &mut scene,
            CollisionContext::empty(false),
            cell,
            false,
            1_000_000,
        ) {
            for hit in scan.flatten() {
                amount = hit
                    .shape
                    .collide(horizon_core::geometry::Axis::Y, entity, amount);
            }
        }
        1.0 + amount
    }
    /// `EntityType.spawn(.., pos, reason, tryMoveDown, false)` for a mob.
    pub(super) fn spawn_mob_at(&mut self, kind: &str, p: Pos, try_move_down: bool) -> Option<u32> {
        let name = if kind.contains(':') {
            kind.to_owned()
        } else {
            format!("minecraft:{kind}")
        };
        // Ender Dragons are simulated separately from `Mob` and therefore do
        // not have a mob-type registry entry for the generic dispenser path.
        if name == "minecraft:ender_dragon" {
            // `EntityType.getYOffset` uses the created entity's full 16 by 8
            // bounding box when the dispenser asks to move it down.
            let y_off = if try_move_down {
                self.drop_offset(16.0, 8.0, p)
            } else {
                0.0
            };
            let at = [
                f64::from(p.0) + 0.5,
                f64::from(p.1) + y_off,
                f64::from(p.2) + 0.5,
            ];
            return self.spawn_dragon_spawned(
                at,
                super::creatures::SpawnSite::Finalizing(
                    horizon_client::mobs::spawn_rules::Reason::Dispenser,
                ),
            );
        }
        let t = self.res.mob_types().get(&name)?.clone();
        let y_off = if try_move_down {
            self.drop_offset(t.width, t.height, p)
        } else {
            0.0
        };
        let at = [
            f64::from(p.0) + 0.5,
            f64::from(p.1) + y_off,
            f64::from(p.2) + 0.5,
        ];
        let id = self.spawn_mob(
            &name,
            at,
            true,
            super::creatures::SpawnSite::Finalizing(
                horizon_client::mobs::spawn_rules::Reason::Dispenser,
            ),
        )?;
        Some(id)
    }
    /// `Player.setItemSlot` of a dispensed piece for the player in place:
    /// `onEquipItem` plays the sound and the event; the modifiers follow.
    fn put_on_player(&mut self, slot: Slot, one: &ItemStack) {
        let name = match slot.armor_index() {
            Some(i) => {
                self.armor[i] = Some(one.clone());
                ["head", "chest", "legs", "feet"][i]
            }
            None => {
                self.offhand = Some(one.clone());
                "offhand"
            }
        };
        self.player_equipped(name, Some(one));
        self.apply_equipment();
    }
    /// `EquipmentDispenseItemBehavior.dispenseEquipment`: the first living
    /// entity in front that `canEquipWithDispenser` gets one of the item.
    fn dispense_equipment(&mut self, pos: Pos, facing: Direction, stack: &mut ItemStack) -> bool {
        // The stack's own `equippable`: its slot and allowed entities, and `dispensable`.
        let Some(equippable) = equippable(self.items(), stack) else {
            return false;
        };
        let default = (self.items().get(&stack.item)).is_some_and(|info| info.equip_dispensable);
        let dispensable = horizon_client::item_sync::dispensable(stack, default);
        let Some(slot) = Slot::from_name(equippable.slot).filter(|_| dispensable) else {
            return false;
        };
        // Equippable.equipSound defaults to the generic armour sound.
        let sound = (self.items().get(&stack.item))
            .and_then(|info| info.equip_sound.clone())
            .or_else(|| Some("minecraft:item.armor.equip_generic".into()));
        let front = offset(pos, facing);
        let one = ItemStack {
            count: 1,
            ..stack.clone()
        };
        // `Equippable.canBeEquippedBy`, then the item's parts (`canEquip`) with the stack converted
        // once, for the entity in front.
        let targets = self.targets_at(front);
        let observed = mod_api::__private::item_listens(mod_api::HookId::CanEquip, &stack.item);
        let mut source_slot = observed.then(|| Some(stack.clone()));
        let mut source = source_slot.as_mut().map(|slot| {
            horizon_client::mobs::equip::EquipDecision::new(stack as *const _ as usize as u64, slot)
        });
        let kinds = if observed {
            targets
                .iter()
                .map(|target| match target {
                    Target::Mob(i) => self.mobs[*i].kind.id.as_str(),
                    Target::Stand(_) => "minecraft:armor_stand",
                    _ => "minecraft:player",
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let entities = if observed {
            targets
                .iter()
                .filter_map(|target| match target {
                    Target::Player => Some(self.own_entity_id() as u32),
                    Target::Guest(conn) => self
                        .lan
                        .as_ref()?
                        .hub
                        .guest(*conn)
                        .map(|guest| guest.entity_id as u32),
                    Target::Stand(i) => Some(self.armor_stands[*i].id),
                    Target::Mob(i) => Some(self.mobs[*i].id),
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let input = source.as_deref().map_or(&*stack, |source| source);
        let (tags, prepared) = (
            &self.stations().enchantments.entity_tags,
            self.equip_stack(input, &kinds, &entities),
        );
        let stamp = source.as_ref().and_then(|source| source.stamp());
        let allows = |entity: u32, id: &str| {
            let data = equippable.allows(id, tags);
            self.can_equip(&prepared, equippable.slot, entity, id, data)
        };
        // LivingEntity.canEquipWithDispenser: alive, the slot usable, the
        // item's allowed entities, the slot empty and the dispenser rule.
        let target = targets.into_iter().find(|t| match *t {
            // Player.canUseSlot: every slot; the armour and the off hand are
            // the ones an equippable item can reach.
            Target::Player => {
                allows(self.own_entity_id() as u32, "minecraft:player")
                    && match slot {
                        Slot::OffHand => self.offhand.is_none(),
                        _ => slot.armor_index().is_some_and(|i| self.armor[i].is_none()),
                    }
            }
            Target::Guest(conn) => self
                .lan
                .as_ref()
                .and_then(|h| h.hub.guest(conn))
                .is_some_and(|g| {
                    allows(g.entity_id as u32, "minecraft:player")
                        && match slot {
                            Slot::OffHand => g.offhand.is_none(),
                            _ => slot.armor_index().is_some_and(|i| g.armor[i].is_none()),
                        }
                }),
            // ArmorStand.canUseSlot: not a locked slot, and a hand needs arms.
            Target::Stand(i) => slot.stand_index().is_some_and(|s| {
                let stand = &self.armor_stands[i];
                allows(stand.id, "minecraft:armor_stand")
                    && !stand.is_disabled(s)
                    && stand.equipment[s].is_none()
            }),
            Target::Mob(i) => {
                let m = &self.mobs[i];
                m.alive()
                    && mob_can_use_slot(m, slot)
                    && allows(m.id, &m.kind.id)
                    && mob_slot_item(m, slot).is_none()
                    && mob_can_dispenser_equip(m, slot)
            }
        });
        let Some(target) = target else {
            return false;
        };
        let entity = match target {
            Target::Player => Some(self.own_entity_id() as u32),
            Target::Guest(conn) => self
                .lan
                .as_ref()
                .and_then(|lan| lan.hub.guest(conn))
                .map(|guest| guest.entity_id as u32),
            Target::Stand(i) => Some(self.armor_stands[i].id),
            Target::Mob(i) => Some(self.mobs[i].id),
        };
        if !entity.is_some_and(|id| self.equip_can_apply(&prepared, id)) {
            return false;
        }
        if let Some(source) = source.as_mut() {
            if !stamp.is_some_and(|stamp| source.matches(stamp)) {
                return false;
            }
            *source.count_mut().expect("live dispenser source") -= 1;
            stack.count = source.count;
        } else {
            stack.count -= 1;
        }
        // setItemSlot: onEquipItem plays the equip sound and the equip event.
        let (at, sound) = match target {
            Target::Guest(conn) => {
                // The guest's own turn: their state in place, so the slot, the
                // sound, the game event and the equipment sync run as for the host.
                let done = self
                    .for_player(super::commands::Target::Guest(conn), |g| {
                        g.put_on_player(slot, &one)
                    })
                    .is_some();
                if !done {
                    stack.count += 1;
                    return false;
                }
                self.native_block_dispense_armor(
                    self.dim.id(),
                    [pos.0, pos.1, pos.2],
                    &stack.item,
                    "player",
                );
                return true;
            }
            Target::Player => {
                self.put_on_player(slot, &one);
                self.native_block_dispense_armor(
                    self.dim.id(),
                    [pos.0, pos.1, pos.2],
                    &stack.item,
                    "player",
                );
                return true;
            }
            Target::Stand(i) => {
                let stand = &mut self.armor_stands[i];
                if let Some(s) = slot.stand_index() {
                    stand.equipment[s] = Some(one);
                }
                (Some(stand.position), sound)
            }
            Target::Mob(i) => {
                let m = &mut self.mobs[i];
                mob_set_slot(m, slot, one);
                // setGuaranteedDrop / setPersistenceRequired.
                m.persistent = true;
                let sound = if m.silent {
                    None
                } else {
                    equip_sound(m, slot, sound)
                };
                (Some(m.position), sound)
            }
        };
        if let Some(at) = at {
            if let Some(sound) = sound {
                self.audio.play(&sound, Some([at.x, at.y, at.z]), 1.0, 1.0);
            }
            let who = match target {
                Target::Mob(i) => Some(horizon_client::mobs::Who::Mob(self.mobs[i].id)),
                Target::Player | Target::Guest(_) | Target::Stand(_) => None,
            };
            let ctx = self.who_context(who, None);
            self.post_game_event("minecraft:equip", [at.x, at.y, at.z], ctx);
        }
        self.native_block_dispense_armor(
            self.dim.id(),
            [pos.0, pos.1, pos.2],
            &stack.item,
            match target {
                Target::Stand(_) => "armor_stand",
                Target::Mob(_) => "mob",
                Target::Player | Target::Guest(_) => "player",
            },
        );
        true
    }
    /// After `EntityType.spawn` / `MobBucketItem.spawn`: `mob.playAmbientSound()`.
    fn mob_ambient(&mut self, id: u32) -> Result<()> {
        if self.dragon_spawn_ambient(id) {
            return Ok(());
        }
        let Some(i) = self.mobs.iter().position(|m| m.id == id) else {
            return Ok(());
        };
        let _save_lease = horizon_client::join_seq::visibility::PassLease::enter();
        let mut m = self.mobs.extract(self.mobs[i].join).unwrap();
        let mut events = Vec::new();
        {
            let w = self.mob_world_base(&[]);
            horizon_client::mobs::ai::ambient_sound(&mut m, &w, &mut events);
        }
        self.mobs.put_back(i, m);
        self.handle_mob_events(events)
    }
    /// A mob a bucket emptied (`MobBucketItem.spawn`): the bucket's components
    /// and entity data (`loadFromBucketTag`), then `setFromBucket(true)`, which
    /// `requiresCustomPersistence` reads, then it calls out. Only the bucket's
    /// data can make it persistent, not the spawn: `spawn_mob_at` creates a
    /// persistent mob. A tadpole's setter does nothing: its `fromBucket` is
    /// always true.
    pub(super) fn mob_placed(&mut self, id: u32, stack: &ItemStack) -> Result<()> {
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            m.persistent = false;
            horizon_client::entity_nbt::apply_item_components(m, stack);
            match m.species() {
                Species::Cod
                | Species::Salmon
                | Species::Pufferfish
                | Species::TropicalFish
                | Species::Axolotl => m.extra.from_bucket = true,
                Species::SulfurCube => {
                    // SulfurCube.loadFromBucketTag: its own lowercase keys, 0
                    // and false when absent, whatever `Age` the data holds.
                    let data = stack
                        .data()
                        .and_then(|c| c.other("minecraft:bucket_entity_data"));
                    let age = data.and_then(|d| horizon_client::save::number(d.get("age")));
                    m.extra.age = age.unwrap_or(0) as i32;
                    m.extra.age_locked =
                        data.is_some_and(|d| horizon_client::entity_nbt::flag(d, "age_locked"));
                    m.extra.scratch[3] = 1;
                }
                _ => {}
            }
        }
        self.mob_ambient(id)
    }
    /// `SpawnEggItemBehavior.execute`: spawn the type selected by the stack's entity data.
    fn dispense_spawn_egg(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let (kind, _) = horizon_client::block_entity_item::spawn_egg_on_spawner(&stack, false);
        let front = offset(pos, facing);
        let Some(id) = self.spawn_mob_at(&kind, front, facing != Direction::Up) else {
            return Ok((Some(stack), CLICK));
        };
        // EntityType.create: `finalizeSpawn` first, the egg's components after.
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            // SpawnEggItemBehavior's `EntityType.spawn` never calls
            // `setPersistenceRequired`, as a spawn egg used by hand doesn't;
            // only the elder guardian's and copper golem's constructors do.
            m.persistent = matches!(m.species(), Species::ElderGuardian | Species::CopperGolem);
            horizon_client::entity_nbt::apply_item_components(m, &stack);
        }
        // EntityType.updateCustomEntityTag: a configured egg
        // (`/give @s pig_spawn_egg[minecraft:entity_data={…}]`) spawns the mob
        // it describes from a dispenser too, not only from a player's hand.
        self.apply_spawn_egg_entity_data(id, &stack);
        self.mob_ambient(id)?;
        // SpawnEggItemBehavior: the entity was placed by the dispenser.
        self.post_block_game_event("minecraft:entity_place", [pos.0, pos.1, pos.2], None, None);
        Ok((shrink(stack), CLICK))
    }

    // ---- ProjectileDispenseBehavior ----

    fn dispense_projectile(&mut self, pos: Pos, facing: Direction, stack: ItemStack) -> Dispensed {
        let item = stack.item.to_string();
        let s = step(facing);
        let dir = [f64::from(s[0]), f64::from(s[1]), f64::from(s[2])];
        // ProjectileItem.DispenseConfig per item: position, uncertainty, power, event.
        let (at, uncertainty, power, event) = match item.as_str() {
            "minecraft:firework_rocket" => (
                Self::dispense_position(pos, facing, 0.500_009_999_999_747_4, [0.0; 3]),
                1.0,
                0.5,
                1004,
            ),
            "minecraft:fire_charge" => (
                Self::dispense_position(pos, facing, 1.0, [0.0; 3]),
                6.666_666_5,
                1.0,
                1018,
            ),
            "minecraft:wind_charge" => (
                Self::dispense_position(pos, facing, 1.0, [0.0; 3]),
                6.666_666_5,
                1.0,
                1051,
            ),
            "minecraft:experience_bottle"
            | "minecraft:splash_potion"
            | "minecraft:lingering_potion" => (
                Self::dispense_position(pos, facing, 0.7, [0.0, 0.1, 0.0]),
                3.0,
                1.375,
                LAUNCH,
            ),
            _ => (
                Self::dispense_position(pos, facing, 0.7, [0.0, 0.1, 0.0]),
                6.0,
                1.1,
                LAUNCH,
            ),
        };
        self.shoot_projectile_item(at, dir, &stack, power, uncertainty);
        (shrink(stack), event)
    }

    /// `ProjectileItem.asProjectile` and `Projectile.spawnProjectileUsingShoot`:
    /// one of `stack` fired from `at` along `dir`.
    pub(super) fn shoot_projectile_item(
        &mut self,
        at: Vec3,
        dir: [f64; 3],
        stack: &ItemStack,
        power: f32,
        uncertainty: f32,
    ) {
        let item = stack.item.to_string();
        let one = ItemStack {
            count: 1,
            ..stack.clone()
        };
        // A rocket makes its own random in `launch_angled_rocket`: the constructor's draws
        // and then the shot's spread come from that one.
        if item == "minecraft:firework_rocket" {
            self.launch_angled_rocket(at, &one, None, |own| {
                let mut shot = Projectile::new(ProjectileKind::Snowball, at, None);
                shot.shoot(own, dir, power, uncertainty);
                shot.velocity
            });
            return;
        }
        // Projectile.shoot spreads the shot with the projectile's own random.
        match item.as_str() {
            "minecraft:experience_bottle"
            | "minecraft:splash_potion"
            | "minecraft:lingering_potion" => {
                let mut shot = super::potions::thrown_projectile(one, at, false);
                shot.shoot_own(dir, power, uncertainty);
                self.add_projectile(shot);
            }
            _ => {
                let kind = match item.as_str() {
                    "minecraft:arrow" => ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: None,
                    },
                    "minecraft:spectral_arrow" => ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: Some(("minecraft:glowing", 200)),
                    },
                    "minecraft:tipped_arrow" => ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: tipped_effect(stack),
                    },
                    "minecraft:fire_charge" | "minecraft:wind_charge" => {
                        // asProjectile aims with the level's random first; the
                        // shot along the facing then replaces that motion.
                        let r = &mut self.world.random;
                        let _ = (
                            r.triangle(dir[0], 0.114_85),
                            r.triangle(dir[1], 0.114_85),
                            r.triangle(dir[2], 0.114_85),
                        );
                        if item == "minecraft:wind_charge" {
                            ProjectileKind::WindCharge
                        } else {
                            ProjectileKind::SmallFireball
                        }
                    }
                    "minecraft:egg" | "minecraft:blue_egg" | "minecraft:brown_egg" => {
                        ProjectileKind::Egg
                    }
                    _ => ProjectileKind::Snowball,
                };
                let mut shot = Projectile::new(kind, at, None);
                shot.set_egg_item(&one);
                // ArrowItem/SpectralArrowItem.asProjectile: dispensed arrows
                // may be picked up.
                if let Some(a) = shot.arrow.as_mut() {
                    a.set_item(&one);
                    a.pickup = horizon_client::arrows::Pickup::Allowed;
                }
                shot.shoot_own(dir, power, uncertainty);
                self.spawn_projectile(shot, &one);
            }
        }
    }

    // ---- entities placed by items ----

    fn dispense_armor_stand(&mut self, pos: Pos, facing: Direction, stack: ItemStack) -> Dispensed {
        let front = offset(pos, facing);
        let foot = Vec3::new(
            f64::from(front.0) + 0.5,
            f64::from(front.1),
            f64::from(front.2) + 0.5,
        );
        // EntityType.create rolls a yaw from the level random before the stack config turns it.
        let _ = self.world.random.next_float();
        self.next_entity += 1;
        let (_, uuid) = horizon_client::mobs::living_random();
        self.persist.uuids.insert(self.next_entity, uuid);
        let stand =
            horizon_client::entities::ArmorStand::new(self.next_entity, foot, y_rot(facing));
        self.world.modified.insert((front.0 >> 4, front.2 >> 4));
        self.armor_stands.push(stand);
        // EntityType.appendDefaultStackConfig: the dispenser's yaw first, then
        // the item's createDefaultStackConfig with no user.
        self.apply_item_stack_config(
            super::commands::Target::Stand(self.next_entity),
            "minecraft:armor_stand",
            &stack,
            false,
        );
        (shrink(stack), CLICK)
    }
    /// A tame donkey, mule or llama in front without a chest gets it.
    fn dispense_chest(&mut self, pos: Pos, facing: Direction, stack: ItemStack) -> Dispensed {
        let front = offset(pos, facing);
        for target in self.targets_at(front) {
            let Target::Mob(i) = target else { continue };
            let m = &mut self.mobs[i];
            if matches!(
                m.species(),
                Species::Donkey | Species::Mule | Species::Llama | Species::TraderLlama
            ) && m.alive()
                && m.extra.tame
                && !m.extra.chested
            {
                m.extra.chested = true;
                set_success("chest", true);
                return (shrink(stack), CLICK);
            }
        }
        (
            self.default_execute(pos, facing, stack),
            optional_event("chest"),
        )
    }
    /// `BoatDispenseItemBehavior`.
    fn dispense_boat(&mut self, pos: Pos, facing: Direction, stack: ItemStack) -> Dispensed {
        let Some((wood, chest)) =
            super::boats::any_boat_wood(&stack.item).map(|(w, c)| (w.to_owned(), c))
        else {
            return (self.dispense_default_in_level(pos, facing, stack), CLICK);
        };
        let front = offset(pos, facing);
        let s = step(facing);
        let c = Self::center(pos);
        // Boat width 1.375.
        let outside = 0.5625 + 1.375 / 2.0;
        let (x, y, z) = (
            c[0] + f64::from(s[0]) * outside,
            c[1] + f64::from(s[1]) * f64::from(1.125f32),
            c[2] + f64::from(s[2]) * outside,
        );
        let below = (front.0, front.1 - 1, front.2);
        let y_offset = if self.water_at(front) {
            1.0
        } else if self.res.facts().is_air(self.block_at(front)) && self.water_at(below) {
            0.0
        } else {
            return (self.dispense_default_in_level(pos, facing, stack), CLICK);
        };
        if !self.boat_models.has(&wood, chest) {
            return (self.dispense_default_in_level(pos, facing, stack), CLICK);
        }
        self.next_entity += 1;
        let mut boat = super::boats::Boat::new(
            self.next_entity,
            wood.as_str().into(),
            Vec3::new(x, y + y_offset, z),
            y_rot(facing),
        );
        if chest {
            boat = boat.with_chest();
        }
        self.world.modified.insert((front.0 >> 4, front.2 >> 4));
        let (id, kind, yaw) = (boat.id, boat.entity_type(), boat.yaw);
        self.boats.push(boat);
        // BoatDispenseItemBehavior: createDefaultStackConfig with no user,
        // then setYRot(direction.toYRot()).
        self.apply_item_stack_config(super::commands::Target::Boat(id), &kind, &stack, false);
        self.set_boat_yaw(id, yaw);
        if self.serving.is_some() || self.options.dedicated.is_some() {
            let boat = self.boats.last().unwrap();
            self.native_vehicle_create(
                boat.entity_type().as_str(),
                [boat.position.x, boat.position.y, boat.position.z],
            );
        }
        (shrink(stack), CLICK)
    }
    /// `MinecartDispenseItemBehavior`.
    fn dispense_minecart(&mut self, pos: Pos, facing: Direction, stack: ItemStack) -> Dispensed {
        let Some(kind) = super::minecarts::CartKind::from_id(&stack.item) else {
            return (self.dispense_default_in_level(pos, facing, stack), CLICK);
        };
        let s = step(facing);
        let c = Self::center(pos);
        let (x, base_y, z) = (
            c[0] + f64::from(s[0]) * 1.125,
            c[1].floor() + f64::from(s[1]),
            c[2] + f64::from(s[2]) * 1.125,
        );
        let front = offset(pos, facing);
        let y_offset = if let Some(rail) = self.rail(None, front.0, front.1, front.2) {
            if rail.shape.is_slope() { 0.6 } else { 0.1 }
        } else {
            if !self.res.facts().is_air(self.block_at(front)) {
                return (self.dispense_default_in_level(pos, facing, stack), CLICK);
            }
            let below = (front.0, front.1 - 1, front.2);
            let Some(rail) = self.rail(None, below.0, below.1, below.2) else {
                return (self.dispense_default_in_level(pos, facing, stack), CLICK);
            };
            if facing != Direction::Down && rail.shape.is_slope() {
                -0.4
            } else {
                -0.9
            }
        };
        self.next_entity += 1;
        let cart = super::minecarts::Minecart::new(
            self.next_entity,
            kind,
            Vec3::new(x, base_y + y_offset, z),
        );
        self.world.modified.insert((front.0 >> 4, front.2 >> 4));
        self.carts.push(cart);
        // AbstractMinecart.createMinecart: createDefaultStackConfig with no user.
        let id = self.next_entity;
        self.apply_item_stack_config(super::commands::Target::Cart(id), kind.id(), &stack, false);
        if self.serving.is_some() || self.options.dedicated.is_some() {
            let cart = self.carts.last().unwrap();
            self.native_vehicle_create(
                cart.kind.id(),
                [cart.position.x, cart.position.y, cart.position.z],
            );
        }
        (shrink(stack), CLICK)
    }

    // ---- buckets ----

    /// The empty bucket: `BucketPickup.pickupBlock`.
    fn dispense_empty_bucket(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = Self::pos_of(offset(pos, facing));
        let picked =
            self.with_level(|b, level, ticks| b.bucket_pickup(level, ticks, target, false));
        self.process_level_events()?;
        match picked {
            Some(filled) => {
                self.post_block_game_event(
                    "minecraft:fluid_pickup",
                    [target.x, target.y, target.z],
                    None,
                    None,
                );
                let filled = ItemStack::new(filled, 1);
                Ok((
                    self.consume_with_remainder(pos, facing, stack, filled),
                    CLICK,
                ))
            }
            None => Ok((self.default_execute(pos, facing, stack), CLICK)),
        }
    }

    // ---- fire, bone meal and TNT ----

    fn dispense_flint_and_steel(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let target_pos = Self::pos_of(target);
        let state = self.block_at(target);
        let name = self.dispenser_block_name(target);
        let mut success = true;
        // tryIgniteExplosiveEntities: the first sulfur cube that can explode is primed.
        let cube = self.targets_at(target).into_iter().find_map(|t| match t {
            Target::Mob(i) => {
                let m = &self.mobs[i];
                (m.species() == Species::SulfurCube
                    && m.alive()
                    && !sulfur_cube::primed(m)
                    && sulfur_cube::archetype(m).is_some_and(|a| a.explosion.is_some()))
                .then_some(i)
            }
            _ => None,
        });
        if let Some(i) = cube {
            // primeTime(false): the fuse is lit only while TNT explodes; the
            // ignition counts either way.
            if self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::TNT_EXPLODES)
                && sulfur_cube::prime(&mut self.mobs[i], false)
            {
                let m = &mut self.mobs[i];
                let (at, id) = (m.position, m.id);
                let mut events = Vec::new();
                m.voice("entity.tnt.primed", &mut events);
                self.handle_mob_events(events)?;
                let ctx = self.who_context(Some(horizon_client::mobs::Who::Mob(id)), None);
                self.post_game_event("minecraft:prime_fuse", [at.x, at.y, at.z], ctx);
            }
            return Ok((self.hurt_and_break(stack, 1), CLICK));
        }
        if self
            .with_level(|b, level, ticks| b.fire_can_be_placed_at(level, ticks, target_pos, facing))
        {
            if !self.native_block_ignite_allowed(target_pos, "flint_and_steel") {
                return Ok((Some(stack), CLICK));
            }
            let fire =
                self.with_level(|b, level, ticks| b.fire_state_for(level, ticks, target_pos));
            self.level_set([target.0, target.1, target.2], fire)?;
            self.post_block_game_event(
                "minecraft:block_place",
                [target.0, target.1, target.2],
                None,
                None,
            );
        } else if self.can_light(state, &name) {
            if let Some(lit) = self.with_props(state, &[("lit", "true")]) {
                self.level_set([target.0, target.1, target.2], lit)?;
            }
            self.post_block_game_event(
                "minecraft:block_change",
                [target.0, target.1, target.2],
                None,
                None,
            );
        } else if &*name == "minecraft:tnt" {
            if self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::TNT_EXPLODES)
                && self.tnt_prime_allowed(target_pos)
            {
                self.spawn_primed_tnt(target_pos, false, None, None);
                self.with_level(|b, level, ticks| b.remove_block(level, ticks, target_pos, false));
                self.process_level_events()?;
            } else {
                success = false;
            }
        }
        let event = if success { CLICK } else { FAIL };
        if success {
            return Ok((self.hurt_and_break(stack, 1), event));
        }
        Ok((Some(stack), event))
    }
    /// `CampfireBlock.canLight || CandleBlock.canLight || CandleCakeBlock.canLight`.
    pub(super) fn can_light(&self, state: BlockStateId, name: &str) -> bool {
        let lit = self.prop(state, "lit");
        let waterlogged = self.prop(state, "waterlogged");
        let dry_unlit = lit.as_deref() == Some("false") && waterlogged.as_deref() == Some("false");
        ((self.in_block_tag("minecraft:campfires", name)
            || self.in_block_tag("minecraft:candles", name))
            && dry_unlit)
            || (self.in_block_tag("minecraft:candle_cakes", name)
                && lit.as_deref() == Some("false"))
    }
    fn dispense_bone_meal(
        &mut self,
        pos: Pos,
        facing: Direction,
        mut stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let target_pos = Self::pos_of(target);
        let ask = self.bonemeal(target_pos, false, &mut stack);
        let grown = self.with_level(|b, level, ticks| {
            ask.crop
                .unwrap_or_else(|| b.grow_crop(level, ticks, target_pos, ask.check))
                || b.grow_water_plant(level, ticks, target_pos, None)
        });
        if grown {
            self.level_events
                .push(horizon_world::level::LevelEvent::LevelEvent {
                    id: 1505,
                    pos: target_pos,
                    data: 15,
                    source: None,
                });
        }
        self.process_level_events()?;
        // A canceled `BonemealEvent` that succeeded uses none of the meal.
        let left = if grown && ask.crop != Some(true) {
            shrink(stack)
        } else {
            (stack.count > 0).then_some(stack)
        };
        Ok((left, if grown { CLICK } else { FAIL }))
    }
    fn dispense_tnt(&mut self, pos: Pos, facing: Direction, stack: ItemStack) -> Dispensed {
        if !self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::TNT_EXPLODES)
        {
            set_success("tnt", false);
            return (Some(stack), FAIL);
        }
        let front = offset(pos, facing);
        if self.sulfur_cube_swallow(front, &stack) {
            return (shrink(stack), optional_event("tnt"));
        }
        let target = Self::pos_of(front);
        if !self.tnt_prime_allowed(target) {
            set_success("tnt", false);
            return (Some(stack), FAIL);
        }
        set_success("tnt", true);
        // new PrimedTnt(level, x + 0.5, y, z + 0.5, null), its sound and ENTITY_PLACE.
        let rot = self.world.random.next_double() * f64::from(std::f32::consts::TAU);
        self.add_primed_tnt(target, rot, 80, None, None);
        self.audio.play(
            "minecraft:entity.tnt.primed",
            Some([
                f64::from(target.x) + 0.5,
                f64::from(target.y),
                f64::from(target.z) + 0.5,
            ]),
            1.0,
            1.0,
        );
        self.post_block_game_event(
            "minecraft:entity_place",
            [front.0, front.1, front.2],
            None,
            None,
        );
        (shrink(stack), CLICK)
    }

    // ---- blocks placed by dispensers ----

    fn dispense_wither_skull(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let free = self.res.facts().is_air(self.block_at(target))
            && target.1 >= self.world.bottom_y() + 2
            && self.ui.difficulty != 0
            && self.can_build_wither(target);
        if free {
            let rotation = match facing {
                Direction::Up | Direction::Down => 0,
                other => {
                    // RotationSegment.convertToSegment: the 2D data value * 4.
                    (match other {
                        Direction::South => 0,
                        Direction::West => 1,
                        Direction::North => 2,
                        _ => 3,
                    }) * 4
                }
            };
            let skull = self
                .pack
                .assets
                .registry
                .default_state("minecraft:wither_skeleton_skull")
                .and_then(|s| self.with_props(s, &[("rotation", &rotation.to_string())]));
            if let Some(skull) = skull {
                self.level_set([target.0, target.1, target.2], skull)?;
                if let Some(entity) = BlockEntity::for_block("minecraft:wither_skeleton_skull") {
                    self.world.block_entities.insert(target, entity);
                }
                self.post_block_game_event(
                    "minecraft:block_place",
                    [target.0, target.1, target.2],
                    None,
                    None,
                );
                self.check_wither_spawn([target.0, target.1, target.2])?;
                return Ok((shrink(stack), CLICK));
            }
        }
        let mut stack = stack;
        let equipped = self.dispense_equipment(pos, facing, &mut stack);
        Ok((
            Some(stack).filter(|s| s.count > 0),
            if equipped { CLICK } else { FAIL },
        ))
    }
    /// `WitherSkullBlock.getOrCreateWitherBase().find(level, pos)`: a soul
    /// sand T for the skull row, in any orientation.
    fn can_build_wither(&self, head: Pos) -> bool {
        self.find_wither_pattern(
            horizon_client::block_pattern::WITHER_BASE,
            [head.0, head.1, head.2],
        )
        .is_some()
    }
    fn dispense_carved_pumpkin(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        if self.res.facts().is_air(self.block_at(target))
            && self.can_spawn_golem([target.0, target.1, target.2])
        {
            if let Some(pumpkin) = self
                .pack
                .assets
                .registry
                .default_state("minecraft:carved_pumpkin")
            {
                let head = [target.0, target.1, target.2];
                self.set_golem_head(head, pumpkin)?;
                self.post_block_game_event("minecraft:block_place", head, None, None);
            }
            return Ok((shrink(stack), CLICK));
        }
        let mut stack = stack;
        let equipped = self.dispense_equipment(pos, facing, &mut stack);
        Ok((
            Some(stack).filter(|s| s.count > 0),
            if equipped { CLICK } else { FAIL },
        ))
    }
    /// `ShulkerBoxDispenseBehavior`: place it like a `DirectionalPlaceContext`.
    fn dispense_shulker_box(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let front = offset(pos, facing);
        let below_empty = self
            .res
            .facts()
            .is_air(self.block_at((front.0, front.1 - 1, front.2)));
        let clicked_face = if below_empty { facing } else { Direction::Up };
        let ctx = PlaceContext {
            clicked_pos: Self::pos_of(front),
            face: clicked_face,
            hit: Vec3::new(
                f64::from(front.0) + 0.5,
                f64::from(front.1),
                f64::from(front.2) + 0.5,
            ),
            inside: false,
            yaw: place_rotation(facing),
            pitch: 0.0,
            sneaking: false,
            item: stack.item.to_string(),
            can_use_game_master_blocks: false,
        };
        // DirectionalPlaceContext places at the front block or nowhere.
        if !self.with_level(|b, level, ticks| b.directional_can_place(level, ticks, &ctx)) {
            return Ok((Some(stack), FAIL));
        }
        if mod_api::__private::listened(mod_api::HookId::EntityPlace)
            && let Some(preview) =
                self.with_level(|b, level, ticks| b.placement_preview(level, ticks, &ctx))
        {
            let against = ctx.placed_against_pos(preview.pos);
            let against = self.world.block(against.x, against.y, against.z);
            let mut canceled = false;
            self.native_entity_place(
                None,
                preview.pos,
                preview.state,
                against,
                mod_api::Placement::Single,
                &mut canceled,
            );
            if canceled {
                return Ok((Some(stack), FAIL));
            }
        }
        let outcome = self.with_level(|b, level, ticks| b.place_block(level, ticks, &ctx));
        let PlaceOutcome::Placed { pos: placed, state } = outcome else {
            self.process_level_events()?;
            return Ok((Some(stack), FAIL));
        };
        let target = (placed.x, placed.y, placed.z);
        let state = self.apply_item_block_state([target.0, target.1, target.2], state, &stack)?;
        if !self.world.block_entities.stateful_entity(state.0)
            && let Some(entity) = self.new_block_entity_for_state(state)
        {
            self.world.block_entities.insert(target, entity);
        }
        if self.world.block_entities.get(&target).is_some() {
            self.apply_placed_item([target.0, target.1, target.2], &stack, false);
        }
        self.block_item_placed(&stack, None, placed, &ctx);
        self.process_level_events()?;
        Ok((shrink(stack), CLICK))
    }

    // ---- bottles, glowstone, shears, brush, honeycomb ----

    fn dispense_glass_bottle(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let state = self.block_at(target);
        let name = self.dispenser_block_name(target);
        let honey = self
            .prop(state, "honey_level")
            .and_then(|v| v.parse::<i32>().ok());
        if self.in_block_tag("minecraft:beehives", &name)
            && let Some(level) = honey
            && level >= 5
        {
            // releaseBeesAndResetHoneyLevel.
            if let Some(reset) = self.with_props(state, &[("honey_level", "0")]) {
                self.level_set([target.0, target.1, target.2], reset)?;
            }
            self.dispenser_empty_hive(target, state);
            let bottle = ItemStack::new("minecraft:honey_bottle", 1);
            self.post_block_game_event("minecraft:fluid_pickup", [pos.0, pos.1, pos.2], None, None);
            return Ok((
                self.consume_with_remainder(pos, facing, stack, bottle),
                CLICK,
            ));
        }
        if self.water_at(target) {
            let water = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water");
            self.post_block_game_event("minecraft:fluid_pickup", [pos.0, pos.1, pos.2], None, None);
            return Ok((
                self.consume_with_remainder(pos, facing, stack, water),
                CLICK,
            ));
        }
        Ok((self.default_execute(pos, facing, stack), FAIL))
    }
    fn dispense_glowstone(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let state = self.block_at(target);
        if &*self.dispenser_block_name(target) == "minecraft:respawn_anchor" {
            let charge = self
                .prop(state, "charges")
                .and_then(|v| v.parse::<i32>().ok())
                .unwrap_or(0);
            if charge != 4 {
                if let Some(next) =
                    self.with_props(state, &[("charges", &(charge + 1).to_string())])
                {
                    self.level_set([target.0, target.1, target.2], next)?;
                    // RespawnAnchorBlock.charge: Context.of(null, newState).
                    self.post_block_game_event(
                        "minecraft:block_change",
                        [target.0, target.1, target.2],
                        None,
                        Some(next),
                    );
                }
                self.audio.play(
                    "minecraft:block.respawn_anchor.charge",
                    Some(Self::center(target)),
                    1.0,
                    1.0,
                );
                return Ok((shrink(stack), CLICK));
            }
            return Ok((Some(stack), FAIL));
        }
        Ok((self.default_execute(pos, facing, stack), CLICK))
    }
    fn dispense_shears(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let sheared = self.shear_beehive(target)? || self.shear_entity(target)?;
        if sheared {
            return Ok((self.hurt_and_break(stack, 1), CLICK));
        }
        Ok((Some(stack), FAIL))
    }
    /// The bees of a hive a dispenser harvested come out (`BEE_RELEASED`:
    /// not at night nor in the rain), through the front of its old state.
    fn dispenser_empty_hive(&mut self, target: Pos, old: BlockStateId) {
        let facing = self.prop(old, "facing");
        self.empty_hive(
            [target.0, target.1, target.2],
            horizon_client::beehive::facing_step(facing.as_deref()),
            horizon_client::beehive::Release::BeeReleased,
        );
    }
    fn shear_beehive(&mut self, target: Pos) -> Result<bool> {
        let state = self.block_at(target);
        let name = self.dispenser_block_name(target);
        let level = self
            .prop(state, "honey_level")
            .and_then(|v| v.parse::<i32>().ok());
        if !self.in_block_tag("minecraft:beehives", &name) || level.is_none_or(|l| l < 5) {
            return Ok(false);
        }
        self.audio.play(
            "minecraft:block.beehive.shear",
            Some(Self::center(target)),
            1.0,
            1.0,
        );
        // BeehiveBlock.dropHoneycomb (harvest/beehive), then
        // releaseBeesAndResetHoneyLevel and the shear event.
        let p = [target.0, target.1, target.2];
        for stack in self.sequence_loot(
            "minecraft:harvest/beehive",
            "minecraft:harvest/beehive",
            None,
        ) {
            self.pop_resource(p, stack);
        }
        if let Some(reset) = self.with_props(state, &[("honey_level", "0")]) {
            self.level_set(p, reset)?;
        }
        self.dispenser_empty_hive(target, state);
        self.post_block_game_event("minecraft:shear", p, None, None);
        Ok(true)
    }
    /// `LootTable.getRandomItems` of `table` in the context of `parent`: the
    /// draws come from `parent`'s server random sequence when it names one (a
    /// nested `loot_table` entry shares its parent's context), else the level's.
    pub(super) fn sequence_loot(
        &mut self,
        parent: &str,
        table: &str,
        held: Option<&ItemStack>,
    ) -> Vec<ItemStack> {
        match world_items!(self).loot_sequence(parent) {
            Some(sequence) => {
                let random = self.rand_sequences.get(&sequence, self.level.seed);
                world_items!(self).loot_table_from(table, false, held, random)
            }
            None => world_items!(self).loot_table(table, false, held, &mut self.world.random),
        }
    }
    /// `Entity.spawnAtLocation(level, stack, offset)`: an item at the entity's
    /// feet plus the offset, moving as `new ItemEntity` makes it.
    pub(super) fn spawn_at_location(
        &mut self,
        at: Vec3,
        y_offset: f64,
        stack: ItemStack,
    ) -> [f64; 3] {
        self.spawn_item_own(stack, [at.x, at.y + y_offset, at.z], 10)
    }
    /// `MushroomCow.shear`: `player` is who sheared it, none for a dispenser.
    pub(super) fn shear_mooshroom(&mut self, index: usize, player: Option<u128>, tool: &ItemStack) {
        let old = self.mobs[index].clone();
        let at = old.position;
        let height = f64::from(old.height());
        let table = if old.extra.variant == 1 {
            "minecraft:shearing/mooshroom/brown"
        } else {
            "minecraft:shearing/mooshroom/red"
        };
        // shear: level.playSound(null, this, MOOSHROOM_SHEAR, source, 1, 1),
        // PLAYERS for a player's shears and BLOCKS for a dispenser's.
        let source = if player.is_some() {
            SoundSource::Players
        } else {
            SoundSource::Blocks
        };
        let shear = LevelSound::new(
            "minecraft:entity.mooshroom.shear",
            [at.x, at.y, at.z],
            1.0,
            1.0,
        )
        .in_source(source);
        self.play_bound_sound(shear, old.id, None);
        self.convert_to(
            &old,
            "minecraft:cow",
            horizon_client::mobs::conversion::Params {
                keep_equipment: false,
                preserve_can_pick_up_loot: false,
            },
            |game, _| {
                game.mooshroom_shear_puff([at.x, at.y, at.z], height);
                let mut drops =
                    game.sequence_loot("minecraft:shearing/mooshroom", table, Some(tool));
                game.native_use_drops(
                    table,
                    [at.x, at.y, at.z],
                    None,
                    Some((old.id, "minecraft:mooshroom")),
                    player,
                    Some(tool),
                    &mut drops,
                );
                for stack in drops {
                    for _ in 0..stack.count {
                        game.spawn_item_own(
                            ItemStack {
                                count: 1,
                                ..stack.clone()
                            },
                            [at.x, at.y + height, at.z],
                            0,
                        );
                    }
                }
            },
        );
    }
    /// `ShearsDispenseItemBehavior.tryShearEntity`: the first live, ready
    /// `Shearable` in the block is sheared (an entity part's `IShearable` answer comes first).
    pub(super) fn shear_entity(&mut self, target: Pos) -> Result<bool> {
        let shears = ItemStack::new("minecraft:shears", 1);
        for t in self.targets_at(target) {
            // Entity.shearOffAllLeashConnections comes first, for any entity.
            if self.shear_off_leashes(t) {
                return Ok(true);
            }
            let Target::Mob(i) = t else { continue };
            let part = self.part_shearable(i, None, &shears);
            let m = &self.mobs[i];
            if !m.alive() || part.is_some_and(|(_, shearable)| !shearable) {
                continue;
            }
            let (at, entity_id, kind) = (m.position, m.id, m.kind.id.clone());
            let sound_at = Some([at.x, at.y, at.z]);
            // shear(level, BLOCKS, tool): level.playSound(null, this, sound, BLOCKS, 1, 1).
            let shear_sound = |name: &'static str| {
                LevelSound::new(name, [at.x, at.y, at.z], 1.0, 1.0).in_source(SoundSource::Blocks)
            };
            match m.species() {
                _ if let Some((part, _)) = part => self.part_shear(i, part, None, &shears),
                Species::SulfurCube if sulfur_cube::has_body(m) => {
                    // SulfurCube.shear: the swallowed block comes out at the
                    // passenger attachment, and nothing is swallowed for 5 s.
                    let height = f64::from(m.height());
                    let id = m.id;
                    if let Some(body) = self.mobs[i].take_body_equipment() {
                        self.spawn_at_location(at, height, body);
                    }
                    // setItemSlot(BODY, EMPTY): an unequip.
                    let ctx = self.who_context(Some(horizon_client::mobs::Who::Mob(id)), None);
                    self.post_game_event("minecraft:unequip", [at.x, at.y, at.z], ctx);
                    self.mobs[i].extra.ints[3] = sulfur_cube::PICKUP_TIMER;
                    self.audio
                        .play("minecraft:entity.sulfur_cube.eject", sound_at, 1.0, 1.0);
                }
                Species::Sheep if !m.extra.sheared && !m.is_baby() => {
                    let color =
                        horizon_client::mobs::render::DYE_NAMES[m.extra.color as usize & 15];
                    self.play_bound_sound(
                        shear_sound("minecraft:entity.sheep.shear"),
                        entity_id,
                        None,
                    );
                    let drops = self.sequence_loot(
                        "minecraft:shearing/sheep",
                        &format!("minecraft:shearing/sheep/{color}"),
                        Some(&shears),
                    );
                    for stack in drops {
                        for _ in 0..stack.count {
                            let one = ItemStack {
                                count: 1,
                                ..stack.clone()
                            };
                            self.spawn_at_location(at, 1.0, one);
                            // Sheep.shear nudges each item with the sheep's own random.
                            let r = &mut self.mobs[i].random;
                            let push = [
                                f64::from((r.next_float() - r.next_float()) * 0.1),
                                f64::from(r.next_float() * 0.05),
                                f64::from((r.next_float() - r.next_float()) * 0.1),
                            ];
                            if let Some(item) = self.just_spawned_item() {
                                item.velocity = Vec3::new(
                                    item.velocity.x + push[0],
                                    item.velocity.y + push[1],
                                    item.velocity.z + push[2],
                                );
                            }
                        }
                    }
                    self.mobs[i].extra.sheared = true;
                }
                Species::Bogged if !m.extra.sheared => {
                    let height = f64::from(m.height());
                    self.play_bound_sound(
                        shear_sound("minecraft:entity.bogged.shear"),
                        entity_id,
                        None,
                    );
                    let drops = self.sequence_loot(
                        "minecraft:shearing/bogged",
                        "minecraft:shearing/bogged",
                        Some(&shears),
                    );
                    for stack in drops {
                        self.spawn_at_location(at, height, stack);
                    }
                    self.mobs[i].extra.sheared = true;
                }
                Species::SnowGolem if !m.extra.sheared => {
                    let eye = m.eye_height();
                    self.play_bound_sound(
                        shear_sound("minecraft:entity.snow_golem.shear"),
                        entity_id,
                        None,
                    );
                    self.mobs[i].extra.sheared = true;
                    let drops = self.sequence_loot(
                        "minecraft:shearing/snow_golem",
                        "minecraft:shearing/snow_golem",
                        Some(&shears),
                    );
                    for stack in drops {
                        self.spawn_at_location(at, eye, stack);
                    }
                }
                // CopperGolem.shear: the antenna's flower drops 1.5 up.
                Species::CopperGolem if self.golem_shearable(i) => {
                    self.play_bound_sound(
                        shear_sound("minecraft:entity.copper_golem.shear"),
                        entity_id,
                        None,
                    );
                    let antenna = self.mobs[i].saddle_item();
                    let id = self.mobs[i].id;
                    // setItemSlot(EQUIPMENT_SLOT_ANTENNA, EMPTY): an unequip.
                    self.mobs[i].set_saddle(None);
                    let ctx = self.who_context(Some(horizon_client::mobs::Who::Mob(id)), None);
                    self.post_game_event("minecraft:unequip", [at.x, at.y, at.z], ctx);
                    if let Some(antenna) = antenna {
                        self.spawn_at_location(at, 1.5, antenna);
                    }
                }
                Species::Mooshroom if !m.is_baby() => {
                    self.shear_mooshroom(i, None, &shears);
                }
                _ => continue,
            }
            self.native_block_shear_entity(
                self.dim.id(),
                [target.0, target.1, target.2],
                entity_id,
                &kind,
            );
            self.post_block_game_event(
                "minecraft:shear",
                [target.0, target.1, target.2],
                None,
                None,
            );
            return Ok(true);
        }
        Ok(false)
    }
    /// `Entity.shearOffAllLeashConnections(null)`: the entity's own lead and
    /// every lead held by it (within a 32 block box around its centre) drop;
    /// a shear game event at the entity and a snip at its block follow.
    fn shear_off_leashes(&mut self, t: Target) -> bool {
        use horizon_client::mobs::{Who, leash::Holder};
        let (holder, centre, own) = match t {
            Target::Mob(i) => {
                let m = &self.mobs[i];
                let b = m.bounds();
                (Holder::Mob(m.id), b.center(), Some(i))
            }
            Target::Player => {
                let Some(v) = self.player_view() else {
                    return false;
                };
                (Holder::Player, v.bounds.center(), None)
            }
            Target::Guest(_) | Target::Stand(_) => return false,
        };
        let area = Aabb::new(centre, centre).inflate(Vec3::new(16.0, 16.0, 16.0));
        let mut leashed: Vec<(u32, usize)> = self
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                !m.removed && m.extra.leash == Some(holder) && overlaps(m.bounds(), area)
            })
            .map(|(j, m)| (m.id, j))
            .collect();
        leashed.sort_unstable();
        let own_leashed = own.is_some_and(|i| self.mobs[i].extra.leash.is_some());
        if leashed.is_empty() && !own_leashed {
            return false;
        }
        // Leashable.dropLeash: a lead item at the leashed entity's feet.
        let drop = |game: &mut Self, j: usize| {
            if horizon_client::mobs::leash::remove(&mut game.mobs[j]).is_some() {
                let at = game.mobs[j].position;
                game.spawn_at_location(at, 0.0, ItemStack::new("minecraft:lead", 1));
            }
        };
        if let Some(i) = own.filter(|_| own_leashed) {
            drop(self, i);
        }
        for (_, j) in leashed {
            drop(self, j);
        }
        let (at, who) = match t {
            Target::Mob(i) => {
                let m = &self.mobs[i];
                (m.position, Who::Mob(m.id))
            }
            _ => (
                self.player.as_ref().map_or(Vec3::ZERO, |p| p.position),
                Who::Player,
            ),
        };
        let ctx = self.who_context(Some(who), None);
        self.post_game_event("minecraft:shear", [at.x, at.y, at.z], ctx);
        let block = [at.x.floor() + 0.5, at.y.floor() + 0.5, at.z.floor() + 0.5];
        self.audio
            .play("minecraft:item.shears.snip", Some(block), 1.0, 1.0);
        true
    }
    /// The brush behaviour: the first armadillo in front that `brushOffScute`s.
    fn dispense_brush(&mut self, pos: Pos, facing: Direction, stack: ItemStack) -> Dispensed {
        let target = offset(pos, facing);
        let armadillos: Vec<usize> = self
            .targets_at(target)
            .into_iter()
            .filter_map(|t| match t {
                Target::Mob(i) if self.mobs[i].species() == Species::Armadillo => Some(i),
                _ => None,
            })
            .collect();
        for i in armadillos {
            if self.mobs[i].is_baby() {
                continue;
            }
            let at = self.mobs[i].position;
            let drops = self.sequence_loot(
                "minecraft:brush/armadillo",
                "minecraft:brush/armadillo",
                Some(&stack),
            );
            for d in drops {
                self.spawn_at_location(at, 0.0, d);
            }
            self.audio.play(
                "minecraft:entity.armadillo.brush",
                Some([at.x, at.y, at.z]),
                1.0,
                1.0,
            );
            let who = horizon_client::mobs::Who::Mob(self.mobs[i].id);
            let ctx = self.who_context(Some(who), None);
            self.post_game_event("minecraft:entity_interact", [at.x, at.y, at.z], ctx);
            return (self.hurt_and_break(stack, 16), optional_event("brush"));
        }
        set_success("brush", false);
        (Some(stack), FAIL)
    }
    /// `SulfurCubeBlockDispenseItemBehavior.dispenseBlock`: the first sulfur
    /// cube in the block that `equipItem`s the stack swallows one.
    fn sulfur_cube_swallow(&mut self, target: Pos, stack: &ItemStack) -> bool {
        let cubes: Vec<usize> = self
            .targets_at(target)
            .into_iter()
            .filter_map(|t| match t {
                Target::Mob(i) if self.mobs[i].species() == Species::SulfurCube => Some(i),
                _ => None,
            })
            .collect();
        for i in cubes {
            let m = &self.mobs[i];
            if m.is_baby() || m.extra.body_id() == Some(&*stack.item) {
                continue;
            }
            let (at, height) = (m.position, f64::from(m.height()));
            // The swallowed block comes out on top (the passenger attachment).
            if let Some(old) = self.mobs[i].take_body_equipment() {
                self.spawn_at_location(at, height, old);
            }
            let m = &mut self.mobs[i];
            m.extra.set_body_stack(Some(ItemStack {
                count: 1,
                ..stack.clone()
            }));
            m.persistent = true;
            let (at, id) = (m.position, m.id);
            // setItemSlot: a body item without `equippable` posts UNEQUIP.
            let ctx = self.who_context(Some(horizon_client::mobs::Who::Mob(id)), None);
            self.post_game_event("minecraft:unequip", [at.x, at.y, at.z], ctx);
            self.audio.play(
                "minecraft:entity.sulfur_cube.absorb",
                Some([at.x, at.y, at.z]),
                1.0,
                1.0,
            );
            return true;
        }
        false
    }
    fn dispense_honeycomb(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let state = self.block_at(target);
        // HoneycombItem.getWaxed, as the player's honeycomb waxes.
        let Some(waxed) = self.honeycomb_waxed(state) else {
            return Ok((
                self.default_execute(pos, facing, stack),
                optional_event("honeycomb"),
            ));
        };
        set_success("honeycomb", true);
        self.level_set([target.0, target.1, target.2], waxed)?;
        self.level_events
            .push(horizon_world::level::LevelEvent::LevelEvent {
                id: 3003,
                pos: Self::pos_of(target),
                data: 0,
                source: None,
            });
        self.process_level_events()?;
        self.audio.play(
            "minecraft:item.honeycomb.wax_on",
            Some(Self::center(target)),
            1.0,
            1.0,
        );
        Ok((shrink(stack), CLICK))
    }
    fn dispense_potion(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let water = stack.potion() == Some("minecraft:water");
        let name = self.dispenser_block_name(target);
        if !water || !self.in_block_tag("minecraft:convertible_to_mud", &name) {
            // The inner default behaviour dispenses, then the potion's own
            // events play on top.
            return Ok((self.dispense_default_in_level(pos, facing, stack), CLICK));
        }
        // The server picks the splash positions with the level random.
        for _ in 0..5 {
            let r = &mut self.world.random;
            let p = [
                f64::from(pos.0) + r.next_double(),
                f64::from(pos.1 + 1),
                f64::from(pos.2) + r.next_double(),
            ];
            self.particles
                .add(&ParticleOptions::simple(ParticleType::Splash), p, [0.0; 3]);
        }
        self.audio.play(
            "minecraft:item.bottle.empty",
            Some(Self::center(pos)),
            1.0,
            1.0,
        );
        self.post_block_game_event("minecraft:fluid_place", [pos.0, pos.1, pos.2], None, None);
        if let Some(mud) = self.pack.assets.registry.default_state("minecraft:mud") {
            self.level_set([target.0, target.1, target.2], mud)?;
        }
        let bottle = ItemStack::new("minecraft:glass_bottle", 1);
        Ok((
            self.consume_with_remainder(pos, facing, stack, bottle),
            CLICK,
        ))
    }
}
/// A tipped arrow's lasting effect at an eighth of the potion's duration
/// (`Arrow.doPostHurtEffects`).
pub(super) fn tipped_effect(stack: &ItemStack) -> Option<(&'static str, i32)> {
    stack
        .potion()
        .and_then(horizon_client::brewing::potion)
        .and_then(|def| {
            def.effects
                .iter()
                .find(|(id, _, _)| !horizon_client::brewing::effect(id).is_some_and(|e| e.instant))
                .map(|(id, duration, _)| {
                    (
                        horizon_client::brewing::namespaced(id),
                        (duration / 8).max(1),
                    )
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a dispense probe part heard: which part, and the dispenser it was for.
    #[derive(Clone, Default)]
    struct Heard(std::sync::Arc<std::sync::Mutex<Vec<(&'static str, [i32; 3], u32)>>>);

    impl Heard {
        fn record(&self, part: &'static str, pos: [i32; 3], state: u32) {
            self.0.lock().unwrap().push((part, pos, state));
        }
        fn take(&self) -> Vec<(&'static str, [i32; 3], u32)> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
    }

    /// Answers for the faced block for dirt and cobblestone, whatever the item's own part says.
    struct Front(Heard);
    #[mod_api::horizon_part]
    impl mod_api::BlockPart for Front {
        fn dispense_method(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::DispenseMethodEvent<'_>,
        ) -> Option<mod_api::Dispensed> {
            let state = crate::game::api_values::TryIntoEngine::try_into_engine(&event.state, &())
                .unwrap()
                .0;
            self.0.record("block", event.pos, state);
            let item = event.stack.item();
            let answers = ["minecraft:dirt", "minecraft:cobblestone"]
                .iter()
                .any(|name| item == mod_api::__private::item_key(name).unwrap());
            answers.then_some((None, FAIL))
        }
    }

    /// Answers for the item it is attached to, if the block in front let it.
    struct Item(Heard);
    #[mod_api::horizon_part]
    impl mod_api::ItemPart for Item {
        fn dispense_item(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::DispenseItemEvent<'_>,
        ) -> Option<mod_api::Dispensed> {
            self.0.record("item", event.pos, 0);
            Some((None, CLICK))
        }
    }

    struct Probe(Heard);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Probe {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let stone = r.block_key("minecraft:stone");
            r.block_part(stone, Front(self.0.clone()));
            for item in ["minecraft:stone", "minecraft:cobblestone"] {
                let item = r.item_key(item);
                r.item_part(item, Item(self.0.clone()));
            }
        }
    }

    /// `DispenserBlock.getDispenseMethod`: the block the dispenser faces
    /// answers first and wins for any item; with no answer from it the item's
    /// part decides; with neither, vanilla drops one item as before.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_faced_block_answers_before_the_item_and_vanilla_stands_when_no_part_listens() {
        use horizon_client::chunk::Chunk;
        use std::sync::Arc;

        let heard = Heard::default();
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "dispense_probe",
            Box::new(Probe(heard.clone())),
        )]
        .into();
        super::super::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let air = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:air")
                    .unwrap();
                // The dispenser at (5, 64, 5) faces stone.
                game.world.set(6, 64, 5, stone).unwrap();
                game.audio.capture = Some(Vec::new());
                let dispense = |game: &mut Game<'_>, item: &str| {
                    game.dispense_item((5, 64, 5), Direction::East, ItemStack::new(item, 1))
                        .unwrap()
                };

                // 1. The faced block answers for an item with no part of its own: it wins, and
                //    the slot is emptied with the event it chose.
                assert_eq!(dispense(game, "minecraft:dirt"), None);
                assert_eq!(heard.take(), [("block", [5, 64, 5], stone.0)]);
                assert_eq!(
                    game.audio.capture.as_ref().unwrap().last().unwrap().event,
                    "minecraft:block.dispenser.fail"
                );
                assert!(
                    game.dropped.is_empty(),
                    "the block's answer owns the dispense"
                );
                game.audio.capture.as_mut().unwrap().clear();

                // 2. The faced block answers for an item that has its own part: its answer wins
                //    and the item's part is never asked.
                assert_eq!(dispense(game, "minecraft:cobblestone"), None);
                assert_eq!(heard.take(), [("block", [5, 64, 5], stone.0)]);
                assert_eq!(
                    game.audio.capture.as_ref().unwrap().last().unwrap().event,
                    "minecraft:block.dispenser.fail"
                );
                assert!(game.dropped.is_empty());
                game.audio.capture.as_mut().unwrap().clear();

                // 3. No block answer: the item's part decides.
                assert_eq!(dispense(game, "minecraft:stone"), None);
                assert_eq!(
                    heard.take(),
                    [("block", [5, 64, 5], stone.0), ("item", [5, 64, 5], 0)]
                );
                assert_eq!(
                    game.audio.capture.as_ref().unwrap().last().unwrap().event,
                    "minecraft:block.dispenser.dispense"
                );
                assert!(game.dropped.is_empty());
                game.audio.capture.as_mut().unwrap().clear();

                // 4. Neither the block in front nor the item has a part: vanilla drops one.
                game.world.set(6, 64, 5, air).unwrap();
                assert_eq!(dispense(game, "minecraft:stick"), None);
                assert!(heard.take().is_empty());
                assert_eq!(game.dropped.len(), 1);
                assert_eq!(game.dropped[0].stack, ItemStack::new("minecraft:stick", 1));
                assert_eq!(
                    game.audio.capture.as_ref().unwrap().last().unwrap().event,
                    "minecraft:block.dispenser.dispense"
                );
            });
        });
    }

    /// Dresses only the entity type it is told to: what the part of an iron helmet answers to
    /// `can_equip`, and what it was asked as `(slot, entity, kind, position)`, the position read
    /// from the entity's handle through the read-only `Host`.
    #[derive(Clone)]
    struct Fits {
        accepts: Arc<std::sync::Mutex<&'static str>>,
        asked: Arc<std::sync::Mutex<Vec<(String, u32, String, Option<[f64; 3]>)>>>,
    }
    #[mod_api::horizon_part]
    impl mod_api::ItemPart for Fits {
        fn can_equip(&self, host: &dyn mod_api::Host, event: &mut mod_api::CanEquipEvent<'_>) {
            let at = event.entity.entity().position(host);
            let (slot, kind) = (event.slot.to_owned(), event.kind.to_owned());
            (self.asked.lock().unwrap()).push((slot, event.entity.id(), kind, at));
            event.can_equip = event.kind == *self.accepts.lock().unwrap();
        }
    }
    struct Tailor(Fits);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Tailor {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let helmet = r.item_key("minecraft:iron_helmet");
            r.item_part(helmet, self.0.clone());
        }
    }
    impl Fits {
        fn new(accepts: &'static str) -> Self {
            Self {
                accepts: Arc::new(std::sync::Mutex::new(accepts)),
                asked: Arc::default(),
            }
        }
        fn asked(&self) -> Vec<(String, u32, String, Option<[f64; 3]>)> {
            std::mem::take(&mut *self.asked.lock().unwrap())
        }
        /// Runs `test` with a mod whose part of the iron helmet is this one installed.
        fn installed(&self, test: impl FnOnce()) {
            let tailor = mod_api::ModEntry::new("tailor", Box::new(Tailor(self.clone())));
            let mods: mod_api::Mods = vec![tailor].into();
            crate::entry::install_client_hooks();
            super::super::api_values::install_parts(&mods).unwrap();
            mod_platform::testing::with_mods(mods, test);
        }
    }

    /// One scene per dispense, three blocks apart: a dispenser at (5, 64, 2 + 3 row) facing east,
    /// and in front a zombie that picks up loot (id 10 + row) and an armour stand (id 20 + row),
    /// the zombie first. Dispenses `stack` and answers whether the dispenser equipped it, the
    /// zombie's slot that took it, whether the stand took it, and where the two stand.
    fn dress(
        game: &mut Game<'_>,
        row: u32,
        stack: ItemStack,
    ) -> (bool, Option<Slot>, bool, [f64; 3]) {
        use horizon_core::geometry::Vec3;
        (game.world.chunks).insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let z = 2 + 3 * row as i32;
        let at = Vec3::new(6.5, 64.0, f64::from(z) + 0.5);
        let mut zombie = horizon_client::mobs::Mob::new(10 + row, kind, at, 0.0);
        zombie.extra.pick_up_loot = true;
        game.mobs.push(zombie);
        game.armor_stands
            .push(horizon_client::entities::ArmorStand::new(20 + row, at, 0.0));
        let mut stack = stack;
        let done = game.dispense_equipment((5, 64, z), Direction::East, &mut stack);
        let zombie = game.mobs.iter().find(|m| m.id == 10 + row).unwrap();
        let worn = [Slot::Head, Slot::Chest, Slot::Legs, Slot::Feet]
            .into_iter()
            .find(|slot| mob_slot_item(zombie, *slot).is_some());
        let stand = game.armor_stands.iter().find(|s| s.id == 20 + row).unwrap();
        (
            done,
            worn,
            stand.equipment.iter().any(Option::is_some),
            [at.x, at.y, at.z],
        )
    }

    /// An iron helmet whose `equippable` is removed (`fields` None) or replaced by `fields`.
    fn helmet_with(fields: Option<Vec<(&str, horizon_core::nbt::Tag)>>) -> ItemStack {
        use horizon_core::nbt::Tag;
        let mut helmet = ItemStack::new("minecraft:iron_helmet", 1);
        let (id, value) = match fields {
            Some(fields) => ("minecraft:equippable", Tag::compound(fields)),
            None => ("!minecraft:equippable", Tag::Compound(Default::default())),
        };
        helmet.data_mut().set_other(id, Some(value));
        helmet
    }

    /// `IItemExtension.canEquip` at the dispenser: vanilla dresses the zombie, the first in
    /// front; a part of the iron helmet that accepts only armour stands dresses the stand
    /// instead, and one that accepts only players dresses neither. The part reads where each
    /// one stands through its `Host`. An item with no part is as vanilla, and a handle that
    /// names no live entity is refused without asking the part.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_can_equip_part_decides_which_entity_a_dispenser_dresses() {
        let fits = Fits::new("minecraft:armor_stand");
        fits.installed(|| {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                // No part answers for the chestplate: the component's answer stands.
                let chestplate = ItemStack::new("minecraft:iron_chestplate", 1);
                let (_, worn, stand, _) = dress(game, 0, chestplate);
                assert_eq!((worn, stand), (Some(Slot::Chest), false));
                assert!(fits.asked().is_empty());
                // The helmet's part accepts only armour stands.
                let helmet = ItemStack::new("minecraft:iron_helmet", 1);
                let (_, worn, stand, at) = dress(game, 1, helmet.clone());
                assert_eq!((worn, stand), (None, true));
                let asked = |id, kind: &str| ("head".to_owned(), id, kind.to_owned(), Some(at));
                assert_eq!(
                    fits.asked(),
                    [
                        asked(11, "minecraft:zombie"),
                        asked(21, "minecraft:armor_stand")
                    ]
                );
                // Or only players: neither stands in front.
                *fits.accepts.lock().unwrap() = "minecraft:player";
                let (done, worn, stand, _) = dress(game, 2, helmet.clone());
                assert_eq!((done, worn, stand), (false, None, false));
                assert_eq!(fits.asked().len(), 2);
                // A missing entity is refused, and the part never hears of it.
                let prepared = game.equip_stack(
                    &helmet,
                    &[
                        "minecraft:player",
                        "minecraft:zombie",
                        "minecraft:armor_stand",
                    ],
                    &[],
                );
                assert!(!game.can_equip(&prepared, "head", 9_999, "minecraft:zombie", true));
                assert!(fits.asked().is_empty());
                // With no part to ask, the component's answer stands for any entity.
                let chestplate =
                    game.equip_stack(&ItemStack::new("minecraft:iron_chestplate", 1), &[], &[]);
                assert!(game.can_equip(&chestplate, "chest", 9_999, "minecraft:zombie", true));
            });
        });
    }

    /// The dispenser reads the stack's own `equippable`, with no part installed: a removed
    /// component dispenses nothing onto anyone, an override for the feet dresses the zombie's
    /// feet, `allowed_entities` naming only armour stands skips the zombie for the stand, an
    /// explicitly empty list allows nobody, and `dispensable: false` keeps the dispenser out.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_dispenser_reads_the_stacks_own_equippable() {
        use horizon_core::nbt::Tag;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let slot = |name: &str| ("slot", Tag::String(name.into()));
            let allowed = |list: Vec<Tag>| ("allowed_entities", Tag::List(list));
            let dressed = |game: &mut Game<'_>, row, stack| {
                let (done, worn, stand, _) = dress(game, row, stack);
                (done, worn, stand)
            };
            assert_eq!(dressed(game, 0, helmet_with(None)), (false, None, false));
            assert_eq!(
                dressed(game, 1, helmet_with(Some(vec![slot("feet")]))),
                (true, Some(Slot::Feet), false)
            );
            let stands = vec![Tag::String("armor_stand".into())];
            assert_eq!(
                dressed(
                    game,
                    2,
                    helmet_with(Some(vec![slot("head"), allowed(stands)]))
                ),
                (true, None, true)
            );
            assert_eq!(
                dressed(
                    game,
                    3,
                    helmet_with(Some(vec![slot("head"), allowed(Vec::new())]))
                ),
                (false, None, false)
            );
            let kept = ("dispensable", Tag::Byte(0));
            assert_eq!(
                dressed(game, 4, helmet_with(Some(vec![slot("head"), kept]))),
                (false, None, false)
            );
        });
    }

    /// The LAN guest in front of a dispenser: a part that accepts only players lets the helmet
    /// on, asked for the guest's own entity at its position; one that accepts only armour stands
    /// keeps it off; and with no part, a stack whose `allowed_entities` leaves the player out is
    /// not dispensed onto them.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_can_equip_part_decides_whether_a_dispenser_dresses_a_guest() {
        use horizon_core::nbt::Tag;
        let fits = Fits::new("minecraft:armor_stand");
        fits.installed(|| {
            let mut resources = None;
            let mut game = server_with_guest(&mut resources);
            // Open the world and admit the guest as a join does, so its handle resolves.
            game.native_lifecycle
                .begin_open(mod_api::Authority::IntegratedServer);
            game.reset_entity_residency();
            let guest = game.lan.as_ref().unwrap().hub.guest(2).unwrap().entity_id as u32;
            assert_eq!(
                dispense_at_guest(&mut game, "minecraft:iron_helmet"),
                (false, 2)
            );
            let asked = ("head".to_owned(), guest, "minecraft:player".to_owned());
            let at = Some([8.5, 200.0, 8.5]);
            assert_eq!(
                fits.asked(),
                [(asked.0.clone(), asked.1, asked.2.clone(), at)]
            );
            *fits.accepts.lock().unwrap() = "minecraft:player";
            assert_eq!(
                dispense_at_guest(&mut game, "minecraft:iron_helmet"),
                (true, 1)
            );
            assert_eq!(fits.asked(), [(asked.0, asked.1, asked.2, at)]);
        });
        let mut resources = None;
        let mut game = server_with_guest(&mut resources);
        let zombies = vec![Tag::String("zombie".into())];
        let fields = vec![
            ("slot", Tag::String("head".into())),
            ("allowed_entities", Tag::List(zombies)),
        ];
        let mut stack = helmet_with(Some(fields));
        assert!(!game.dispense_equipment((8, 201, 8), Direction::Down, &mut stack));
        assert!(game.lan.as_ref().unwrap().hub.guest(2).unwrap().armor[0].is_none());
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn bucket_ambient_extraction_keeps_the_bound_mob_owner() {
        use horizon_mod_runtime::attachments::{
            AttachmentType, Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
        };
        use std::sync::{Arc, Mutex};
        static VALUE: AttachmentType<i32> = AttachmentType {
            namespace: "test",
            name: "ambient_lifetime",
            default: || 0,
            codec: None,
            copy_on_death: false,
            sync: false,
        };
        super::super::native_entity_refs::tests::with_game(|game| {
            let kind = game.res.mob_types().get("minecraft:cod").unwrap().clone();
            for id in [70, 71, 72] {
                game.mobs
                    .push(Mob::new(id, kind.clone(), Vec3::new(0.5, 70.0, 0.5), 0.0));
            }
            let holder = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), 71);
            let index = Arc::new(Mutex::new(AttachmentIndex::default()));
            let mut values = Attachments::default();
            VALUE.set(&mut values, Holder::Entity(71), 10).unwrap();
            let original = game.mobs.update(1, |mob| {
                mob.attachment_lifetime().bind(&mut values, &index, holder);
                mob.attachment_lifetime().registration().unwrap()
            });
            let key = ("test".into(), "ambient_lifetime".into());
            let before = index.lock().unwrap().snapshot(&key);
            game.mob_placed(71, &ItemStack::new("minecraft:axolotl_bucket", 1))
                .unwrap();
            assert!(game.mobs[1].extra.from_bucket);
            assert_eq!(
                game.mobs.iter().map(|m| m.id).collect::<Vec<_>>(),
                [70, 71, 72]
            );
            let current = game
                .mobs
                .update(1, |m| m.attachment_lifetime().registration().unwrap());
            assert!(original.same_registration(&current));
            assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
            game.mobs.remove(1);
            assert!(index.lock().unwrap().snapshot(&key).is_empty());
            assert_eq!(before[0].entry.read(&VALUE), Ok(&10));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn dispenser_stack_damage_preserves_break_remainder() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let mut shears = ItemStack::new("minecraft:shears", 2);
            let max = game.items().get(&shears.item).unwrap().max_damage;
            shears.damage = max - 1;
            let remainder = game.hurt_and_break(shears, 1).unwrap();
            assert_eq!(remainder.count, 1);
            assert_eq!(remainder.damage, max);
        });
    }

    /// `IItemExtension.damageItem` on the wear of a tool a dispenser uses: a brush (16 points a
    /// brushing) whose part halves it loses 8, one whose part cancels it keeps its durability, and
    /// the shears, which have no part, lose the whole point.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; process-wide parts, run this test alone"]
    fn a_damage_item_part_changes_the_wear_of_a_dispensed_tool() {
        use crate::game::damage_item_tests::{Rule, with_wear_part};
        with_wear_part("minecraft:brush", Rule::Halve, |game, wear| {
            let damage = |game: &mut Game<'_>, item: &str, points| {
                game.hurt_and_break(ItemStack::new(item, 1), points)
                    .unwrap()
                    .damage
            };
            assert_eq!(damage(game, "minecraft:brush", 16), 8);
            assert_eq!(wear.take_asked(), [16]);
            wear.set(Rule::Cancel);
            assert_eq!(damage(game, "minecraft:brush", 16), 0);
            assert_eq!(wear.take_asked(), [16]);
            assert_eq!(damage(game, "minecraft:shears", 1), 1);
            assert!(wear.take_asked().is_empty());
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn sulfur_cube_dispenser_replacement_drops_the_full_old_stack() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:sulfur_cube")
                .unwrap()
                .clone();
            let mut mob = Mob::new(
                12,
                kind,
                horizon_core::geometry::Vec3::new(0.5, 0.0, 0.5),
                0.0,
            );
            let mut old = ItemStack::new("minecraft:stone", 3);
            old.damage = 7;
            old.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("kept".into()));
            mob.extra.set_body_stack(Some(old.clone()));
            game.mobs.push(mob);
            let incoming = ItemStack::new("minecraft:dirt", 5);
            assert!(game.sulfur_cube_swallow((0, 0, 0), &incoming));
            assert!(game.dropped.iter().any(|drop| drop.stack == old));
            assert_eq!(
                game.mobs[0].equipment_stacks()[6],
                Some(ItemStack::new("minecraft:dirt", 1))
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn mod_dispenser_answers_preserve_fallback_and_own_consumption() {
        use super::super::api_values::{StackReadError, TryIntoApi};
        super::super::native_entity_refs::tests::with_game(|game| {
            assert!(
                mod_dispensed(None, game.items()).is_none(),
                "no registered behaviour keeps vanilla"
            );
            assert_eq!(
                mod_dispensed(Some((None, CLICK)), game.items()),
                Some((None, CLICK))
            );
            let stack = ItemStack::new("minecraft:stone", 7);
            let Some((Some(left), event)) = mod_dispensed(
                Some((Some(stack.try_into_api().unwrap()), FAIL)),
                game.items(),
            ) else {
                panic!("handled stack")
            };
            assert_eq!(left, stack, "the returned remainder is not consumed again");
            assert_eq!(event, FAIL, "the answer chooses its sound");
            // An empty engine value is refused before a mod answer can be constructed.
            assert_eq!(
                ItemStack { count: 0, ..stack }.try_into_api(),
                Err(StackReadError::Empty)
            );
            assert_eq!(
                mod_dispensed(Some((None, 0)), game.items()),
                Some((None, 0))
            );
        });
    }

    use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
    use horizon_protocol::packets::configuration::ClientInformation;
    use std::sync::{Arc, atomic::AtomicI32, mpsc};

    /// A server with no player of its own, an open hub and one guest (conn 2)
    /// standing at (8.5, 200, 8.5) on a stone floor.
    fn server_with_guest(
        resources: &mut Option<std::sync::Arc<crate::game::Resources>>,
    ) -> Game<'_> {
        let mut game = super::super::game_tests::tests::server_game(resources);
        game.build_registries();
        game.set_allow_commands(true);
        game.block_ticks
            .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
        game.lan_host(
            super::super::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "dispenser-equip-test".into(), 2)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 2,
                integrated: false,
                levels: Default::default(),
            },
        )
        .unwrap();
        let (tx, _rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: 2,
                uuid: 208,
                name: "Wearer".into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            2,
            [8.5, 200.0, 8.5],
            2,
        );
        guest.dimension = "minecraft:overworld".into();
        guest.placed = true;
        guest.loaded = true;
        game.lan.as_mut().unwrap().hub.guests.push(guest);
        game
    }

    fn dispense_at_guest(game: &mut Game<'_>, item: &str) -> (bool, u32) {
        let mut stack = ItemStack::new(item, 2);
        // The dispenser sits one block over the guest's feet, facing down.
        let done = game.dispense_equipment((8, 201, 8), Direction::Down, &mut stack);
        (done, stack.count)
    }

    /// `EquipmentDispenseItemBehavior`: a server's player, the integrated
    /// owner or a guest, takes the piece (a server has no player of its own).
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_dispenser_equips_a_server_player_in_front() {
        let mut resources = None;
        let mut game = server_with_guest(&mut resources);
        assert!(game.player.is_none(), "the server thread has no player");
        // The guest's feet are in block (8, 200, 8): the dispenser at y 201
        // facing down targets the block under it.
        let mut stack = ItemStack::new("minecraft:carved_pumpkin", 3);
        assert!(game.dispense_equipment((8, 201, 8), Direction::Down, &mut stack));
        assert_eq!(stack.count, 2);
        let guest = game.lan.as_ref().unwrap().hub.guest(2).unwrap();
        assert_eq!(
            guest.armor[0].as_ref().map(|s| (&*s.item, s.count)),
            Some(("minecraft:carved_pumpkin", 1))
        );
        // The head is taken now: a second pumpkin is not dispensed onto them.
        assert!(!game.dispense_equipment((8, 201, 8), Direction::Down, &mut stack));
        assert_eq!(stack.count, 2);
        // A boots piece still has its own empty slot.
        let (done, left) = dispense_at_guest(&mut game, "minecraft:iron_boots");
        assert_eq!((done, left), (true, 1), "boots go on the free feet slot");
        let guest = game.lan.as_ref().unwrap().hub.guest(2).unwrap();
        assert!(guest.armor[3].is_some() && guest.armor[1].is_none());
    }

    /// A spectator is not a target (`canEquipWithDispenser`: `!isSpectator()`).
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_dispenser_skips_a_spectator_and_a_dead_player() {
        let mut resources = None;
        let mut game = server_with_guest(&mut resources);
        for spectate in [true, false] {
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.game_mode = if spectate { 3 } else { 0 };
                g.dead = !spectate;
            });
            let mut stack = ItemStack::new("minecraft:iron_helmet", 1);
            assert!(!game.dispense_equipment((8, 201, 8), Direction::Down, &mut stack));
            assert_eq!(stack.count, 1);
        }
    }

    /// `ArmorStand.canUseSlot` through `isDisabled`: the first stand in front
    /// that can wear the piece gets it. Bit `1 << filterBit` locks a slot
    /// (head 4, chest 3, legs 2, feet 1); a hand needs arms.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_dispenser_skips_an_armor_stand_whose_slot_is_locked() {
        use horizon_client::entities::ArmorStand;
        let mut resources = None;
        let mut game = server_with_guest(&mut resources);
        game.lan.as_mut().unwrap().hub.guests.clear();
        let at = horizon_core::geometry::Vec3::new(8.5, 200.0, 8.5);
        let mut stands = [(1, 1u32 << 4), (2, 0)].map(|(id, locked)| {
            let mut s = ArmorStand::new(id, at, 0.0);
            s.disabled_slots = locked as i32;
            s
        });
        stands[0].show_arms = true;
        game.armor_stands.clear();
        game.armor_stands.extend(stands.iter().cloned());
        // The head-locked first stand is skipped for a helmet; the second wears it.
        let mut stack = ItemStack::new("minecraft:iron_helmet", 1);
        assert!(game.dispense_equipment((8, 201, 8), Direction::Down, &mut stack));
        assert!(game.armor_stands[0].equipment[5].is_none());
        assert!(game.armor_stands[1].equipment[5].is_some());
        // The same stand takes chest armour: only its head is locked.
        let mut stack = ItemStack::new("minecraft:iron_chestplate", 1);
        assert!(game.dispense_equipment((8, 201, 8), Direction::Down, &mut stack));
        assert!(game.armor_stands[0].equipment[4].is_some());
        // Every stand locked for that slot: nothing takes it (the dispenser ejects it).
        game.armor_stands[0].disabled_slots = 1 << 4;
        game.armor_stands[1].disabled_slots = 1 << 4;
        game.armor_stands[1].equipment[5] = None;
        let mut stack = ItemStack::new("minecraft:iron_helmet", 1);
        assert!(!game.dispense_equipment((8, 201, 8), Direction::Down, &mut stack));
        assert_eq!(stack.count, 1);
    }

    /// A stand of entity id `id` standing at `x`, 200, `z`.
    fn stand_at(id: u32, x: f64, z: f64) -> horizon_client::entities::ArmorStand {
        let at = horizon_core::geometry::Vec3::new(x, 200.0, z);
        horizon_client::entities::ArmorStand::new(id, at, 0.0)
    }

    /// A guest whose client is still on its loading screen (joined or
    /// respawned, no ServerboundPlayerLoadedPacket yet) is in the level all the
    /// same: `getEntitiesOfClass` finds it and it wears the piece.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_dispenser_equips_a_player_whose_client_is_still_loading() {
        let mut resources = None;
        let mut game = server_with_guest(&mut resources);
        game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            (g.loaded, g.loading) = (false, true);
        });
        let (done, left) = dispense_at_guest(&mut game, "minecraft:iron_helmet");
        assert_eq!((done, left), (true, 1));
        let guest = game.lan.as_ref().unwrap().hub.guest(2).unwrap();
        let head = guest.armor[0].as_ref().map(|s| &*s.item);
        assert_eq!(head, Some("minecraft:iron_helmet"));
    }

    /// Players are entities of the level: in one section they go in join
    /// order (entity ids) with the mobs and stands, not after them.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_player_and_a_stand_in_front_go_in_join_order() {
        // (stand id, guest id) -> who wears the helmet: the lower id joined first.
        for (stand_id, guest_id, guest_first) in [(7, 5, true), (3, 5, false)] {
            let mut resources = None;
            let mut game = server_with_guest(&mut resources);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.entity_id = guest_id);
            game.armor_stands.clear();
            game.armor_stands.extend(vec![stand_at(stand_id, 8.5, 8.5)]);
            let (done, _) = dispense_at_guest(&mut game, "minecraft:iron_helmet");
            assert!(done);
            let guest = game.lan.as_ref().unwrap().hub.guest(2).unwrap();
            assert_eq!(
                guest.armor[0].is_some(),
                guest_first,
                "stand {stand_id}, guest {guest_id}"
            );
            assert_eq!(game.armor_stands[0].equipment[5].is_some(), !guest_first);
        }
    }

    /// `forEachAccessibleNonEmptySection` walks sections before join order:
    /// a stand whose feet are in section x 1 but whose box reaches into block
    /// x 15 comes after one in section x 0, even with the lower id.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_dispenser_walks_entity_sections_before_ids() {
        let mut resources = None;
        let mut game = server_with_guest(&mut resources);
        game.lan.as_mut().unwrap().hub.guests.clear();
        // Stand boxes are 0.5 wide: x 16.2 spans 15.95..16.45.
        game.armor_stands.clear();
        game.armor_stands
            .extend(vec![stand_at(1, 16.2, 8.5), stand_at(2, 15.5, 8.5)]);
        let mut stack = ItemStack::new("minecraft:iron_helmet", 1);
        assert!(game.dispense_equipment((15, 201, 8), Direction::Down, &mut stack));
        assert!(game.armor_stands[0].equipment[5].is_none());
        assert!(game.armor_stands[1].equipment[5].is_some());
    }

    /// `SectionPos.asLong` packs z (22 bits) over y (20 bits) under a fixed x,
    /// and the section set is sorted as longs: z -1 packs as 0x3FFFFF << 20,
    /// above z 0's 0; y -1 at z 0 packs as 0xFFFFF, below z 1's 1 << 20.
    #[test]
    fn sections_sort_as_their_packed_keys() {
        use horizon_core::geometry::Vec3;
        let at = |x: f64, y: f64, z: f64| super::section_order(Vec3::new(x, y, z));
        assert!(at(0.0, 0.0, -1.0) > at(0.0, 0.0, 0.0));
        assert!(at(0.0, -1.0, 0.0) < at(0.0, 0.0, 16.0));
        assert!(at(0.0, -1.0, 0.0) > at(0.0, 0.0, 0.0));
        // x is the outer loop, ascending as a signed coordinate.
        assert!(at(-1.0, 0.0, 0.0) < at(0.0, 0.0, 0.0));
        assert!(at(15.9, 0.0, 0.0) < at(16.0, 0.0, 0.0));
    }
}

#[cfg(test)]
#[path = "dispenser/decision_tests.rs"]
mod decision_tests;
