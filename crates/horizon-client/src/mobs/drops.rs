//! What a dying mob leaves behind: `LivingEntity.die`'s drop step. The
//! killer's `killedEntity` hook (a charged creeper's head drop), then
//! `dropAllDeathLoot`: the entity loot table (drawn from the world's random
//! sequence), `Mob.dropCustomDeathLoot` (equipment by drop chance, with the
//! entity random), the species' extra drops (an enderman's carried block) and
//! the experience orbs.
use super::{Mob, Species, equine, gear, raider, render::DYE_NAMES};
use crate::random_sequences::RandomSequences;
use horizon_core::random::{LegacyRandom, LevelRandom, RandomSource};
use horizon_world::{
    loot::{EnchantedLevel, EntityLootParams, LootDamage, LootData, LootEntity, LootStack},
    registry::BlockStateId,
};

/// `DropChances.DEFAULT_EQUIPMENT_DROP_CHANCE`.
pub const DEFAULT_DROP_CHANCE: f32 = 0.085;
/// `EquipmentSlot.VALUES`.
pub use horizon_world::loot::EQUIPMENT_SLOTS as SLOTS;

/// One thing a death leaves in the world, in spawn order.
#[derive(Clone, Debug, PartialEq)]
pub enum Drop {
    Item(LootStack),
    /// A whole stack a mob kept (in an equipment slot, its inventory or a
    /// horse's chest), with its damage, name and components, dropped as it is.
    Stack(crate::items::ItemStack),
    /// `Enderman.dropCustomDeathLoot`: the carried block's own drops, rolled
    /// with `enderman::carried_block_tool` (`enderman::carried_block_roll`).
    CarriedBlock(BlockStateId),
    /// The experience the mob is worth (`getExperienceReward`), before
    /// `ExperienceOrb.award` splits it into orbs.
    Experience(i32),
    /// A custom death drop with an explicit initial item-entity age.
    /// Bypasses stack-only table modifiers; joins the captured living drops.
    Aged(LootStack, i32),
}

/// How the mob died.
pub struct Death<'a> {
    pub loot: &'a LootData,
    /// The killing `DamageSource`.
    pub damage: Option<LootDamage>,
    /// `LAST_DAMAGE_PLAYER`: the player who last hurt the mob, when they are
    /// still in the world. Gone after they leave, even while the memory lasts.
    pub player: Option<LootEntity>,
    /// `LivingEntity.lastHurtByPlayerMemoryTime > 0` (`playerKilled`):
    /// experience and equipment follow this, not whether `player` is present.
    pub player_killed: bool,
    /// That player's luck.
    pub luck: f32,
    /// The entity the mob rides.
    pub vehicle: Option<LootEntity>,
    /// The `doMobLoot` game rule.
    pub mob_drops: bool,
    pub world_seed: i64,
}

/// What a death's loot rolls with: the world's random sequences, the level's
/// random and, when a mod listens, the game's answer to
/// `enchanted_entity_loot`, asked in the middle of a roll.
pub trait DeathRolls {
    fn sequences(&mut self) -> &mut RandomSequences;
    fn level_random(&mut self) -> &mut LegacyRandom;
    /// Whether [`Self::enchanted_level`] is to be asked at all.
    fn asks_enchanted_level(&self) -> bool {
        false
    }
    /// The level a loot table uses of `enchantment`, given the attacker's.
    fn enchanted_level(&mut self, _enchantment: &str, level: i32) -> i32 {
        level
    }
}
/// A death's randoms and nothing else: no mod is asked.
pub struct PlainRolls<'a> {
    pub sequences: &'a mut RandomSequences,
    pub level_random: &'a mut LegacyRandom,
}
impl DeathRolls for PlainRolls<'_> {
    fn sequences(&mut self) -> &mut RandomSequences {
        self.sequences
    }
    fn level_random(&mut self) -> &mut LegacyRandom {
        self.level_random
    }
}

/// `instanceof Monster` (Monster.shouldDropLoot/shouldDropExperience).
pub fn is_monster(s: Species) -> bool {
    use Species as S;
    matches!(
        s,
        S::Blaze
            | S::Bogged
            | S::Breeze
            | S::CaveSpider
            | S::Creaking
            | S::Creeper
            | S::Drowned
            | S::ElderGuardian
            | S::Enderman
            | S::Endermite
            | S::Evoker
            | S::Giant
            | S::Guardian
            | S::Husk
            | S::Illusioner
            | S::Parched
            | S::Piglin
            | S::PiglinBrute
            | S::Pillager
            | S::Ravager
            | S::Silverfish
            | S::Skeleton
            | S::Spider
            | S::Stray
            | S::Vex
            | S::Vindicator
            | S::Warden
            | S::Witch
            | S::Wither
            | S::WitherSkeleton
            | S::Zoglin
            | S::Zombie
            | S::ZombieVillager
            | S::ZombifiedPiglin
    )
}
/// `Mob.xpReward` as the constructors set it.
fn xp_reward(m: &Mob) -> i32 {
    use Species as S;
    match m.species() {
        S::Blaze | S::Breeze | S::Guardian | S::ElderGuardian | S::Evoker => 10,
        S::Vex | S::Endermite => 3,
        S::PiglinBrute | S::Ravager => 20,
        S::Wither => 50,
        S::Creaking => 0,
        // Hoglin.setBaby.
        S::Hoglin if m.is_baby() => 3,
        S::Ghast | S::Hoglin | S::Phantom | S::Shulker => 5,
        // Slime.setSize / MagmaCube.setSize (unsized cubes keep 0).
        S::Slime | S::MagmaCube => m.extra.size.max(0),
        s if is_monster(s) => 5,
        _ => 0,
    }
}
/// The mobs that extend `Animal`.
pub fn is_animal(s: Species) -> bool {
    use Species as S;
    matches!(
        s,
        S::Armadillo
            | S::Axolotl
            | S::Bee
            | S::Camel
            | S::CamelHusk
            | S::Cat
            | S::Chicken
            | S::Cow
            | S::Donkey
            | S::Fox
            | S::Frog
            | S::HappyGhast
            | S::Goat
            | S::Hoglin
            | S::Horse
            | S::Llama
            | S::Mooshroom
            | S::Mule
            | S::Nautilus
            | S::Ocelot
            | S::Panda
            | S::Parrot
            | S::Pig
            | S::PolarBear
            | S::Rabbit
            | S::Sheep
            | S::SkeletonHorse
            | S::Sniffer
            | S::Strider
            | S::TraderLlama
            | S::Turtle
            | S::Wolf
            | S::ZombieHorse
            | S::ZombieNautilus
    )
}
/// AgeableWaterCreature and WaterAnimal.
fn is_water_creature(s: Species) -> bool {
    use Species as S;
    matches!(
        s,
        S::Dolphin
            | S::GlowSquid
            | S::Squid
            | S::Cod
            | S::Salmon
            | S::Pufferfish
            | S::TropicalFish
            | S::Tadpole
    )
}
fn zombie_family(s: Species) -> bool {
    use Species as S;
    matches!(
        s,
        S::Zombie | S::Husk | S::Drowned | S::ZombieVillager | S::ZombifiedPiglin
    )
}
fn is_raider(s: Species) -> bool {
    use Species as S;
    matches!(
        s,
        S::Pillager | S::Vindicator | S::Evoker | S::Illusioner | S::Witch | S::Ravager
    )
}
/// The species whose body item Horizon draws and handles as armour (and whose
/// sun-protection slot is the body): the equines (a horse's armour, a llama's
/// carpet, a camel's harness) and the wolf, happy ghast, nautilus and sulfur cube.
pub fn body_slot_species(s: Species) -> bool {
    super::species::body_slot(s) || equine::is_equine(s)
}

/// The item a mob has in an `EquipmentSlot` (by name), with its enchantments.
pub fn slot_item(m: &Mob, slot: &str) -> Option<LootStack> {
    let (item, index) = match slot {
        "mainhand" => (m.extra.main_hand_id(), Some(gear::MAIN)),
        "offhand" => (m.extra.off_hand_id(), Some(gear::OFF)),
        "head" => (m.extra.head_id(), Some(gear::HEAD)),
        "chest" => (
            m.extra.slot_id(horizon_items::equipment::CHEST),
            Some(gear::CHEST),
        ),
        "legs" => (
            m.extra.slot_id(horizon_items::equipment::LEGS),
            Some(gear::LEGS),
        ),
        "feet" => (
            m.extra.slot_id(horizon_items::equipment::FEET),
            Some(gear::FEET),
        ),
        "body" => (m.extra.body_id(), None),
        "saddle" => {
            let saddle = m.saddle_item()?;
            let mut stack = loot_stack(&saddle.item, 1);
            stack.damage = saddle.damage as i32;
            for (e, l) in saddle.enchantments() {
                stack.enchantments.push((e.to_string(), *l as i32));
            }
            return Some(stack);
        }
        _ => (None, None),
    };
    // A panda keeps the whole stack it picked up in its mouth.
    let count = if slot == "mainhand" && m.species() == Species::Panda {
        super::panda::held_count(m)
    } else {
        1
    };
    let mut stack = loot_stack(item?, count as i32);
    if slot == "body" && m.species() == Species::Wolf {
        stack.damage = m.extra.body_damage();
    }
    if let Some(full) = stored_slot_stack(m, slot) {
        stack.damage = full.damage as i32;
        if m.species() != Species::Panda {
            stack.count = full.count as i32;
        }
    }
    if let (Some(index), Some(g)) = (index, gear::gear(m)) {
        for (e, l) in &g.slots[index] {
            stack.enchantments.push((e.to_string(), *l as i32));
        }
        if let Some(trim) = &g.trims[index] {
            let trim = Some(crate::item_data::trim::to_nbt(trim));
            stack.components.push(("minecraft:trim".to_owned(), trim));
        }
    }
    Some(stack)
}
/// The `finalize` slot of a drop slot's name, for the slots a stack is kept in.
fn slot_index(slot: &str) -> Option<usize> {
    use super::finalize as f;
    Some(match slot {
        "mainhand" => f::MAINHAND,
        "offhand" => f::OFFHAND,
        "head" => f::HEAD,
        "chest" => f::CHEST,
        "legs" => f::LEGS,
        "feet" => f::FEET,
        _ => return None,
    })
}
/// The whole stack the mob kept in `slot` (by name), when it kept one.
fn stored_slot_stack(m: &Mob, slot: &str) -> Option<crate::items::ItemStack> {
    match slot {
        "body" => m.equipment_stacks()[6].clone(),
        "saddle" => m.saddle_item(),
        _ => super::finalize::stored_stack(m, slot_index(slot)?),
    }
}
/// `EnchantmentHelper.has(stack, EnchantmentEffectComponents.PREVENT_EQUIPMENT_DROP)`
/// on a whole stack: whether its enchantments keep it from dropping.
fn stack_prevents_drop(loot: &LootData, stack: &crate::items::ItemStack) -> bool {
    let mut probe = loot_stack(&stack.item, stack.count as i32);
    probe.enchantments = stack
        .enchantments()
        .iter()
        .map(|(e, l)| (e.to_string(), *l as i32))
        .collect();
    loot.prevents_equipment_drop(&probe)
}
/// The drop of what the mob has in `slot`, given its `stack` after the drop
/// rolls: the whole stack the mob kept (with the rolled damage), else the loot stack.
fn slot_drop(m: &Mob, slot: &str, stack: LootStack) -> Drop {
    match stored_slot_stack(m, slot).filter(|_| m.species() != Species::Panda) {
        Some(mut full) => {
            full.damage = stack.damage.max(0) as u32;
            Drop::Stack(full)
        }
        None => Drop::Item(stack),
    }
}
/// `setItemSlot(slot, EMPTY)`.
pub fn clear_slot(m: &mut Mob, slot: &str) {
    let index = match slot {
        "mainhand" => {
            m.extra.set_main_hand(None);
            gear::MAIN
        }
        "offhand" => {
            m.extra
                .set_slot_stack(horizon_items::equipment::OFFHAND, None);
            gear::OFF
        }
        "head" => {
            m.extra.set_slot_stack(horizon_items::equipment::HEAD, None);
            gear::HEAD
        }
        "chest" => {
            m.extra
                .set_slot_stack(horizon_items::equipment::CHEST, None);
            gear::CHEST
        }
        "legs" => {
            m.extra.set_slot_stack(horizon_items::equipment::LEGS, None);
            gear::LEGS
        }
        "feet" => {
            m.extra.set_slot_stack(horizon_items::equipment::FEET, None);
            gear::FEET
        }
        "body" => {
            m.extra.set_body_stack(None);
            return;
        }
        "saddle" => {
            m.set_saddle(None);
            return;
        }
        _ => return,
    };
    if m.extra.gear.is_some() {
        gear::set(m, index, Vec::new());
    }
}
fn loot_stack(item: &str, count: i32) -> LootStack {
    LootStack {
        item: item.to_owned(),
        count,
        damage: 0,
        enchantments: Vec::new(),
        potion: None,
        map: false,
        exploration: None,
        ominous_amplifier: None,
        stew_effects: Vec::new(),
        names: Vec::new(),
        components: Vec::new(),
    }
}
/// `Mob.dropChances.byEquipment(slot)`: the recorded chance, or the one the
/// game's spawn and equip paths give (saddles and body armour are put on by
/// `Equippable` and always drop; a captain's banner always drops; a vex
/// never drops its sword).
pub fn drop_chance(m: &Mob, slot: &str) -> f32 {
    let index = SLOTS.iter().position(|s| *s == slot).unwrap_or(0);
    if let Some(chances) = &m.extra.drop_chances {
        return chances[index];
    }
    match slot {
        // A saddle or a body-slot species' worn armour (put on by `Equippable`)
        // always drops; a body item elsewhere came from a command or a save.
        "saddle" => 2.0,
        "body" if body_slot_species(m.species()) => 2.0,
        "head" if is_raider(m.species()) && m.extra.head_id() == Some(raider::BANNER) => 2.0,
        "mainhand" if m.species() == Species::Vex => 0.0,
        _ => DEFAULT_DROP_CHANCE,
    }
}
/// `DropChances.isPreserved`: whether the slot's drop chance is > 1.0.
pub fn is_preserved(m: &Mob, slot: &str) -> bool {
    drop_chance(m, slot) > 1.0
}
/// `Mob.dropPreservedEquipment`: drops any equipped items whose drop chance is > 1.0.
pub fn drop_preserved_equipment(m: &mut Mob) -> Vec<crate::items::ItemStack> {
    use horizon_core::equipment_slot::EquipmentSlot;
    let mut dropped = Vec::new();
    for &slot in &SLOTS {
        if is_preserved(m, slot) {
            let eq_slot = match slot {
                "mainhand" => Some(EquipmentSlot::MainHand),
                "offhand" => Some(EquipmentSlot::OffHand),
                "feet" => Some(EquipmentSlot::Feet),
                "legs" => Some(EquipmentSlot::Legs),
                "chest" => Some(EquipmentSlot::Chest),
                "head" => Some(EquipmentSlot::Head),
                "body" => Some(EquipmentSlot::Body),
                "saddle" => Some(EquipmentSlot::Saddle),
                _ => None,
            };
            if let Some(eq) = eq_slot
                && let Some(stack) = super::finalize::equipment_stack(m, eq)
                && stack.count > 0
                && &*stack.item != "minecraft:air"
            {
                clear_slot(m, slot);
                dropped.push(stack);
            }
        }
    }
    dropped
}
/// The facts loot predicates read about a mob.
pub fn loot_entity(m: &Mob, vehicle: Option<LootEntity>) -> LootEntity {
    use Species as S;
    let s = m.species();
    let mut e = LootEntity::new(&m.kind.id);
    e.on_fire = m.fire_ticks > 0 && !m.kind.flags.fire_immune;
    e.baby = m.is_baby();
    e.vehicle = vehicle.map(Box::new);
    if matches!(s, S::Slime | S::MagmaCube | S::SulfurCube) {
        // An unsized cube has the synced default size 1.
        e.cube_size = Some(m.extra.size.max(1));
    }
    if s == S::Sheep {
        e.sheep = Some((
            DYE_NAMES[m.extra.color as usize & 15].to_owned(),
            m.extra.sheared,
        ));
    }
    if is_raider(s) {
        e.captain = Some(raider::is_captain(m));
    }
    if s == S::Frog {
        e.frog_variant = Some(
            ["minecraft:temperate", "minecraft:warm", "minecraft:cold"]
                [usize::from(m.extra.variant).min(2)]
            .to_owned(),
        );
    }
    if s == S::Chicken {
        e.chicken_variant = super::finalize::variant_component(m).map(|(_, id)| id);
    }
    e.powered = s == S::Creeper && m.extra.charged;
    e.equipment = SLOTS
        .iter()
        .filter_map(|slot| slot_item(m, slot).map(|i| (slot.to_string(), i)))
        .collect();
    e
}
/// What a mob clears on itself in `LivingEntity.die`, before any loot is
/// rolled: `Wolf.die` drops the wet flag and the shake animation, so a wolf
/// killed mid-shake does not shake on the ground.
pub fn death_state(m: &mut Mob) {
    if m.species() == Species::Wolf {
        super::tame::die(m);
    }
}
/// What `roll_death` collected, for `finish_death`.
pub struct Rolled {
    out: Vec<Drop>,
    skulls: bool,
    this: LootEntity,
    allay_load: Option<(std::sync::Arc<str>, i32)>,
    table_rolled: bool,
}
/// What a mod may change about the chance of one equipment slot
/// (`GetEquipmentDropChanceEvent`), asked before its roll.
pub struct EquipmentChoice<'a> {
    pub slot: usize,
    /// The stack that would drop (the mob's whole stack, damage as it is).
    pub stack: &'a Drop,
    /// `DropChances.byEquipment`, before the enchantments.
    pub original: f32,
    /// Whether the slot is preserved: it drops without a player kill.
    pub preserved: bool,
    /// The chance after the enchantments of the killer and the victim.
    pub chance: f32,
}
/// `LivingEntity.die`'s drops for `m` (equipment it loses is removed from
/// it). Returns the drops in spawn order and whether a charged creeper
/// killer dropped a head (`Creeper.droppedSkulls`).
/// `level_random` is the level's random: what a table rolls with when it has
/// neither a seed nor a random sequence (`LootContext.Builder.create`).
pub fn die(
    m: &mut Mob,
    d: &Death,
    sequences: &mut RandomSequences,
    level_random: &mut LevelRandom,
) -> (Vec<Drop>, bool) {
    let rolled = roll_death(
        m,
        d,
        &mut PlainRolls {
            sequences,
            level_random,
        },
    );
    finish_death(m, d, rolled, None)
}
/// The first half of `die`: `dropFromLootTable`, which uses the world's
/// random sequences, and what comes before it.
pub fn roll_death(m: &mut Mob, d: &Death, rolls: &mut dyn DeathRolls) -> Rolled {
    let mut out = Vec::new();
    let mut table_rolled = false;
    death_state(m);
    let this = loot_entity(m, d.vehicle.clone());
    let mut skulls = false;
    // Creeper.killedEntity: a charged creeper's victim drops its head.
    if let Some(source) = d.damage.as_ref().and_then(|x| x.source.as_ref())
        && source.kind == "minecraft:creeper"
        && source.powered
        && d.mob_drops
    {
        let params = EntityLootParams {
            this: this.clone(),
            damage: d.damage.clone(),
            player: None,
            luck: 0.0,
        };
        let items = roll(d, "minecraft:charged_creeper/root", &params, rolls, 0);
        skulls = items.iter().any(|i| !i.is_empty());
        out.extend(items.into_iter().filter(|i| !i.is_empty()).map(Drop::Item));
    }
    let killed_by_player = d.player_killed;
    let s = m.species();
    // Fox.dropAllDeathLoot: whatever it holds in its mouth comes out first.
    if s == Species::Fox
        && let Some(stack) = slot_item(m, "mainhand")
    {
        out.push(slot_drop(m, "mainhand", stack));
        clear_slot(m, "mainhand");
    }
    // `LivingEntity.dropAllDeathLoot`'s `shouldDropLoot`: the loot table and
    // `dropCustomDeathLoot` only; `dropEquipment` below always runs.
    // Allay: what it carries (`ints[1]` of the held item) is its inventory.
    let allay_load = (s == Species::Allay)
        .then(|| {
            super::finalize::item(m, super::finalize::MAINHAND).map(|item| (item, m.extra.ints[1]))
        })
        .flatten();
    if let Some(table) = death_loot_table(m, d.mob_drops) {
        let params = EntityLootParams {
            this: this.clone(),
            damage: d.damage.clone(),
            player: d.player.clone().filter(|_| killed_by_player),
            luck: if killed_by_player { d.luck } else { 0.0 },
        };
        let items = roll(d, &table, &params, rolls, m.extra.death_table_seed);
        out.extend(items.into_iter().filter(|i| !i.is_empty()).map(Drop::Item));
        table_rolled = true;
    }
    Rolled {
        out,
        skulls,
        this,
        allay_load,
        table_rolled,
    }
}
/// The second half of `die`: `Mob.dropCustomDeathLoot` (each slot's chance,
/// roll and clear, in order, with `choose` asked for a slot just before its
/// roll), the species' extras and the experience.
pub fn finish_death(
    m: &mut Mob,
    d: &Death,
    rolled: Rolled,
    mut choose: Option<&mut dyn FnMut(EquipmentChoice<'_>) -> f32>,
) -> (Vec<Drop>, bool) {
    let Rolled {
        mut out,
        skulls,
        this,
        allay_load,
        table_rolled,
    } = rolled;
    let killed_by_player = d.player_killed;
    let s = m.species();
    if table_rolled {
        // Mob.dropCustomDeathLoot.
        let damage_params = damage_context(this.clone(), d);
        for (index, slot) in SLOTS.iter().enumerate() {
            let stack = slot_item(m, slot);
            let Some((mut chance, preserve)) = slot_chance(m, d, &damage_params, slot) else {
                continue;
            };
            let Some(mut stack) = stack else { continue };
            if let Some(choose) = choose.as_mut() {
                let considered = slot_drop(m, slot, stack.clone());
                chance = choose(EquipmentChoice {
                    slot: index,
                    stack: &considered,
                    original: drop_chance(m, slot),
                    preserved: preserve,
                    chance,
                });
            }
            if d.loot.prevents_equipment_drop(&stack) || !(killed_by_player || preserve) {
                continue;
            }
            if m.random.next_float() >= chance {
                continue;
            }
            let max = d.loot.max_damage(&stack.item);
            if !preserve && max > 0 {
                let inner = m.random.next_int_bound((max - 3).max(1)).unwrap_or(0);
                stack.damage = max - m.random.next_int_bound(1 + inner).unwrap_or(0);
            }
            out.push(slot_drop(m, slot, stack));
            clear_slot(m, slot);
        }
        // Enderman.dropCustomDeathLoot: after Monster's, the carried block.
        if s == Species::Enderman
            && let Some(state) = m.extra.carried
        {
            out.push(Drop::CarriedBlock(state));
        }
        // WitherBoss.dropCustomDeathLoot: the star is spawned, then
        // `setExtendedLifetime` gives it `age = -6000`, so it outlives the
        // 6000-tick age discard by another 6000 ticks.
        if s == Species::Wither {
            out.push(Drop::Aged(loot_stack("minecraft:nether_star", 1), -6000));
        }
    }
    // Allay.dropEquipment: its inventory, then the main-hand item unless it
    // prevents equipment drops.
    if s == Species::Allay {
        if let Some(stack) = m.extra.allay.as_mut().and_then(|st| st.inventory.take()) {
            out.push(Drop::Stack(stack));
        }
        if let Some(stack) = slot_item(m, "mainhand")
            && !d.loot.prevents_equipment_drop(&stack)
        {
            out.push(slot_drop(m, "mainhand", stack));
            clear_slot(m, "mainhand");
        }
    }
    // Allay.dropEquipment: its inventory, then the main-hand item unless it
    // prevents equipment drops.
    if let Some((item, carried)) = allay_load {
        if carried > 0 {
            out.push(Drop::Item(loot_stack(&item, carried)));
        }
        if let Some(stack) = slot_item(m, "mainhand")
            && !d.loot.prevents_equipment_drop(&stack)
        {
            out.push(slot_drop(m, "mainhand", stack));
            clear_slot(m, "mainhand");
        }
    }
    // Piglin.dropCustomDeathLoot, inside `shouldDropLoot` (`table_rolled`):
    // everything in its inventory, as the whole stacks it kept
    // (`removeAllItems`, with no vanishing curse check).
    if s == Species::Piglin && table_rolled {
        for stack in m.extra.inventory.drain(..) {
            if stack.count > 0 {
                out.push(Drop::Stack(stack));
            }
        }
    }
    // dropEquipment: AbstractHorse's inventory, then a chested horse's chest.
    if equine::is_equine(s) {
        for stack in m.extra.items.iter().flatten() {
            if stack.count > 0 && !stack_prevents_drop(d.loot, stack) {
                out.push(Drop::Stack(stack.clone()));
            }
        }
        if m.extra.chested {
            out.push(Drop::Item(loot_stack("minecraft:chest", 1)));
            m.extra.chested = false;
        }
    }
    // dropExperience.
    if killed_by_player && drops_experience(m) && d.mob_drops {
        out.push(Drop::Experience(base_experience(m)));
    }
    (out, skulls)
}
/// The loot parameters of the damage that killed `m`, as the equipment drop
/// chance enchantments read them (no luck, no killing player).
fn damage_context(this: LootEntity, d: &Death) -> EntityLootParams {
    EntityLootParams {
        this,
        damage: d.damage.clone(),
        player: None,
        luck: 0.0,
    }
}
/// `Mob.dropCustomDeathLoot`'s chance for `slot` before the roll, and whether
/// the slot is preserved: none when `dropChances` is 0 (the slot is skipped),
/// else the chance after `EnchantmentHelper.processEquipmentDropChance`.
fn slot_chance(
    m: &Mob,
    d: &Death,
    damage_params: &EntityLootParams,
    slot: &str,
) -> Option<(f32, bool)> {
    let chance = drop_chance(m, slot);
    if chance == 0.0 {
        return None;
    }
    let living_attacker = d
        .damage
        .as_ref()
        .and_then(|x| x.source.as_ref())
        .is_some_and(|a| a.living);
    let after = if living_attacker {
        d.loot.equipment_drop_chance(damage_params, chance)
    } else {
        chance
    };
    Some((after, chance > 1.0))
}
/// The loot table `die` rolls for `m` (`LivingEntity.getLootTable`), when it
/// rolls one: none with `doMobLoot` off, nor for a baby that isn't a monster.
pub fn death_loot_table(m: &Mob, mob_drops: bool) -> Option<String> {
    // Mob.getLootTable: the mob's own `DeathLootTable`, else its type's
    // (`EntityType.getDefaultLootTable`: `entities/` before the type's path,
    // in its namespace, so a content mod's type rolls its own table).
    (mob_drops && (is_monster(m.species()) || !m.is_baby())).then(|| {
        m.extra.death_loot_table.clone().unwrap_or_else(|| {
            let id = m.kind.id.as_str();
            let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
            format!("{namespace}:entities/{path}")
        })
    })
}
/// `LootTable.getRandomItems(params, seed, ...)`: a seed's own random, else
/// the table's random sequence, else the level's random.
fn roll(
    d: &Death,
    table: &str,
    params: &EntityLootParams,
    rolls: &mut dyn DeathRolls,
    seed: i64,
) -> Vec<LootStack> {
    if seed != 0 {
        let random = &mut LegacyRandom::new(seed);
        return if rolls.asks_enchanted_level() {
            let ask = &mut |_: &mut LegacyRandom, id: &str, level| rolls.enchanted_level(id, level);
            d.loot.roll_death_entity(table, random, params, ask)
        } else {
            d.loot.roll_entity(table, random, params)
        };
    }
    match d.loot.random_sequence(table) {
        Some(id) => roll_live(d, table, params, rolls, |r| {
            r.sequences().get(id, d.world_seed)
        }),
        None => roll_live(d, table, params, rolls, |r| r.level_random()),
    }
}
/// A roll on a random the game keeps (`home` finds it), with `roll` given the
/// question to ask at each level it reads. A listener asked in the middle of
/// the roll can reach that random, so it is back home while the listener runs,
/// and the roll carries on from wherever the listener left it.
pub fn roll_asked<R: Clone, T>(
    rolls: &mut dyn DeathRolls,
    home: impl for<'r> Fn(&'r mut dyn DeathRolls) -> &'r mut R,
    roll: impl FnOnce(&mut R, &mut EnchantedLevel<'_, R>) -> T,
) -> T {
    let mut random = home(&mut *rolls).clone();
    let rolled =
        roll(&mut random, &mut |random: &mut R,
                                enchantment: &str,
                                level| {
            *home(&mut *rolls) = random.clone();
            let level = rolls.enchanted_level(enchantment, level);
            *random = home(&mut *rolls).clone();
            level
        });
    *home(rolls) = random;
    rolled
}
/// A death's roll on a random the game keeps: asked when a mod listens.
fn roll_live<R: RandomSource + Clone>(
    d: &Death,
    table: &str,
    params: &EntityLootParams,
    rolls: &mut dyn DeathRolls,
    home: impl for<'r> Fn(&'r mut dyn DeathRolls) -> &'r mut R,
) -> Vec<LootStack> {
    if !rolls.asks_enchanted_level() {
        return d.loot.roll_entity(table, home(rolls), params);
    }
    roll_asked(rolls, home, |random, ask| {
        d.loot.roll_death_entity(table, random, params, ask)
    })
}
/// `LivingEntity.shouldDropExperience`: babies drop none, except monsters and
/// hoglins; tadpoles never do.
pub fn drops_experience(m: &Mob) -> bool {
    match m.species() {
        Species::Tadpole => false,
        Species::Hoglin => true,
        s if is_monster(s) => true,
        _ => !m.is_baby(),
    }
}
/// `getBaseExperienceReward` (no vanilla enchantment changes mob experience).
pub fn base_experience(m: &mut Mob) -> i32 {
    use Species as S;
    let s = m.species();
    match s {
        // Chicken.getBaseExperienceReward.
        S::Chicken if m.extra.jockey => 10,
        S::SulfurCube if m.is_baby() => 0,
        S::SulfurCube => 1 + m.random.next_int_bound(2).unwrap_or(0),
        // Hoglin / Piglin: the reward without the equipment bonus.
        S::Hoglin | S::Piglin => xp_reward(m),
        s if is_animal(s) || is_water_creature(s) => 1 + m.random.next_int_bound(3).unwrap_or(0),
        _ => {
            let mut reward = xp_reward(m);
            // Zombie.getBaseExperienceReward: babies are worth 2.5 times as much.
            if zombie_family(s) && m.is_baby() {
                reward = (reward as f32 * 2.5) as i32;
            }
            if reward > 0 {
                let mut result = reward;
                for slot in SLOTS {
                    // EquipmentSlot.canIncreaseExperience: all but the saddle.
                    if slot != "saddle"
                        && slot_item(m, slot).is_some()
                        && drop_chance(m, slot) <= 1.0
                    {
                        result += 1 + m.random.next_int_bound(3).unwrap_or(0);
                    }
                }
                result
            } else {
                reward
            }
        }
    }
}

#[cfg(test)]
mod death_loot_table_tests {
    use super::*;
    use crate::mobs::world::level_random;
    use horizon_world::loot::{LootData, PackOverlay};

    /// Mob.getLootTable and LootTable.getRandomItems: a mob's own
    /// `DeathLootTable` is rolled instead of its type's, with the level's
    /// random when the table has no random sequence (as a pack's tables
    /// mostly don't), and with its `DeathLootTableSeed` when set.
    #[test]
    fn a_mob_drops_its_own_death_loot_table() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("data").exists() {
            return;
        }
        let table = br#"{"type":"minecraft:entity","pools":[{"rolls":1,
            "entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]}"#;
        let packs = [("loot_table/pack:boss".to_owned(), vec![table.to_vec()])];
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let loot = LootData::load(&root, &PackOverlay::new(&packs), &defaults).unwrap();
        let death = Death {
            loot: &loot,
            damage: None,
            player: None,
            player_killed: false,
            luck: 0.0,
            vehicle: None,
            mob_drops: true,
            world_seed: 0,
        };
        let items = |seed: i64, own: bool| {
            let kind = crate::mobs::sensor_scan_tests::kind();
            let mut m = Mob::new(1, kind, horizon_core::geometry::Vec3::ZERO, 0.0);
            if own {
                m.extra.death_loot_table = Some("pack:boss".into());
            }
            m.extra.death_table_seed = seed;
            let mut sequences = RandomSequences::default();
            let (out, _) = die(&mut m, &death, &mut sequences, &mut level_random(7));
            out.into_iter()
                .filter_map(|d| match d {
                    Drop::Item(s) => Some(s.item),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(items(0, true), ["minecraft:diamond"]);
        assert_eq!(items(42, true), ["minecraft:diamond"]);
        // Without one, the type's table (a cow's beef and leather).
        assert!(items(0, false).iter().all(|i| i != "minecraft:diamond"));
    }

    /// `LootTable.createStackSplitter` on a death, through `die`, with
    /// `table`'s ender pearls.
    fn death_pearls(name: &str, modifier: &str) -> Vec<i32> {
        let root = std::env::temp_dir().join(format!("drops-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("data/minecraft/loot_table");
        std::fs::create_dir_all(&dir).unwrap();
        let table = format!(
            r#"{{"type":"minecraft:entity","pools":[{{"rolls":1,"entries":[{{"type":"minecraft:item",
                "name":"minecraft:ender_pearl","modifier":{modifier}}}]}}]}}"#
        );
        std::fs::write(dir.join("test.json"), table).unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let loot = LootData::load(&root, &PackOverlay::NONE, &defaults).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        let death = Death {
            loot: &loot,
            damage: None,
            player: None,
            player_killed: false,
            luck: 0.0,
            vehicle: None,
            mob_drops: true,
            world_seed: 0,
        };
        let kind = crate::mobs::sensor_scan_tests::kind();
        let mut m = Mob::new(1, kind, horizon_core::geometry::Vec3::ZERO, 0.0);
        m.extra.death_loot_table = Some("minecraft:test".into());
        let mut sequences = RandomSequences::default();
        let (out, _) = die(&mut m, &death, &mut sequences, &mut level_random(7));
        out.into_iter()
            .filter_map(|d| match d {
                Drop::Item(s) if s.item == "minecraft:ender_pearl" => Some(s.count),
                _ => None,
            })
            .collect()
    }

    /// A death drop splits at its own `max_stack_size` patch: 12 ender pearls
    /// made to stack to 5 drop as 5, 5 and 2.
    #[test]
    fn a_death_drop_splits_at_its_own_max_stack_size() {
        let pearls = death_pearls(
            "max-stack",
            r#"[{"type":"minecraft:set_count","count":12},
                {"type":"minecraft:set_components","components":{"minecraft:max_stack_size":5}}]"#,
        );
        assert_eq!(pearls, [5, 5, 2]);
    }

    /// The standard removal patch (`"!minecraft:max_stack_size": {}`) leaves a
    /// stack size of 1: three pearls drop one by one.
    #[test]
    fn a_death_drop_without_a_max_stack_size_drops_one_by_one() {
        let pearls = death_pearls(
            "no-max-stack",
            r#"[{"type":"minecraft:set_count","count":3},
                {"type":"minecraft:set_components","components":{"!minecraft:max_stack_size":{}}}]"#,
        );
        assert_eq!(pearls, [1, 1, 1]);
    }

    /// A listener of `enchanted_entity_loot` draws from the level's random in
    /// the middle of a roll on that random: the roll carries on after its
    /// draw, as vanilla's one shared random does.
    #[test]
    fn a_draw_made_while_asked_keeps_its_place_in_the_roll() {
        struct Drawing<'a>(PlainRolls<'a>, bool);
        impl DeathRolls for Drawing<'_> {
            fn sequences(&mut self) -> &mut RandomSequences {
                self.0.sequences()
            }
            fn level_random(&mut self) -> &mut LegacyRandom {
                self.0.level_random()
            }
            fn asks_enchanted_level(&self) -> bool {
                true
            }
            fn enchanted_level(&mut self, _: &str, level: i32) -> i32 {
                if self.1 {
                    self.0.level_random.next_int();
                }
                level
            }
        }
        let root = std::env::temp_dir().join(format!("loot-drops-{}", std::process::id()));
        let dir = root.join("data/minecraft/loot_table");
        std::fs::create_dir_all(&dir).unwrap();
        let table = r#"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item",
            "name":"minecraft:stone","modifier":[
            {"type":"minecraft:enchanted_count_increase","enchantment":"minecraft:looting",
             "count":0.0},
            {"type":"minecraft:set_count",
             "count":{"type":"minecraft:uniform","min":1.0,"max":60.0}}]}]}]}"#;
        std::fs::write(dir.join("test.json"), table).unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let loot = LootData::load(&root, &PackOverlay::NONE, &defaults).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        let death = Death {
            loot: &loot,
            damage: None,
            player: None,
            player_killed: false,
            luck: 0.0,
            vehicle: None,
            mob_drops: true,
            world_seed: 0,
        };
        let run = |mut level_random: LegacyRandom, draws: bool| {
            let mut sequences = RandomSequences::default();
            let mut rolls = Drawing(
                PlainRolls {
                    sequences: &mut sequences,
                    level_random: &mut level_random,
                },
                draws,
            );
            let params = EntityLootParams::default();
            let items = roll(&death, "minecraft:test", &params, &mut rolls, 0);
            (items[0].count, level_random.next_long())
        };
        // The draw comes before the roll's only one, as if made just before it.
        let mut ahead = LegacyRandom::new(7);
        ahead.next_int();
        assert_eq!(run(LegacyRandom::new(7), true), run(ahead, false));
    }

    /// `GetEquipmentDropChanceEvent`: a slot is asked at its own point of
    /// `Mob.dropCustomDeathLoot` (with the stack, the original chance and the
    /// preserved flag), the chance a mod chooses replaces vanilla's, and the
    /// roll is drawn from the mob's random either way.
    #[test]
    fn a_chosen_equipment_drop_chance_replaces_the_roll_only() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("data").exists() {
            return;
        }
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let loot = LootData::load(&root, &PackOverlay::new(&[]), &defaults).unwrap();
        let death = Death {
            loot: &loot,
            damage: None,
            player: Some(LootEntity::new("minecraft:player")),
            player_killed: true,
            luck: 0.0,
            vehicle: None,
            mob_drops: true,
            world_seed: 0,
        };
        let run = |chosen: Option<f32>| {
            let kind = crate::mobs::sensor_scan_tests::kind();
            let mut m = Mob::new(1, kind, horizon_core::geometry::Vec3::ZERO, 0.0);
            m.extra.set_slot_id(
                horizon_items::equipment::MAINHAND,
                Some("minecraft:stick".into()),
            );
            m.extra.set_slot_id(
                horizon_items::equipment::FEET,
                Some("minecraft:apple".into()),
            );
            let mut sequences = RandomSequences::default();
            let rolled = roll_death(
                &mut m,
                &death,
                &mut PlainRolls {
                    sequences: &mut sequences,
                    level_random: &mut LegacyRandom::new(7),
                },
            );
            let mut asked = Vec::new();
            let mut ask = |c: EquipmentChoice<'_>| {
                let stack = match c.stack {
                    Drop::Item(s) => s.item.to_string(),
                    other => format!("{other:?}"),
                };
                asked.push((c.slot, stack, c.original, c.preserved, c.chance));
                chosen.unwrap_or(c.chance)
            };
            let (out, _) = finish_death(&mut m, &death, rolled, Some(&mut ask));
            let dropped = |item: &str| {
                out.iter()
                    .any(|d| matches!(d, Drop::Item(s) if s.item == item))
            };
            (dropped("minecraft:stick"), asked, m.random.next_float())
        };
        let (always, asked, after_always) = run(Some(1.0));
        let (never, _, after_never) = run(Some(0.0));
        let (_, _, after_default) = run(None);
        // Asked per item-holding slot, in order, before its roll.
        assert_eq!(
            asked
                .iter()
                .map(|a| (a.0, a.1.as_str(), a.2, a.3, a.4))
                .collect::<Vec<_>>(),
            [
                (
                    0,
                    "minecraft:stick",
                    DEFAULT_DROP_CHANCE,
                    false,
                    DEFAULT_DROP_CHANCE
                ),
                (
                    2,
                    "minecraft:apple",
                    DEFAULT_DROP_CHANCE,
                    false,
                    DEFAULT_DROP_CHANCE
                ),
            ]
        );
        assert!(always && !never);
        // The mob's random is where vanilla's roll leaves it, whatever is chosen.
        assert_eq!(after_always, after_never);
        assert_eq!(after_never, after_default);
    }
}
#[cfg(test)]
mod slot_tests {
    use super::*;
    use crate::mobs::sensor_scan_tests::kind;
    use crate::mobs::world::level_random;
    use horizon_core::geometry::Vec3;
    use horizon_world::loot::PackOverlay;
    use std::sync::Arc;

    fn mob(id: &str, species: Species) -> Mob {
        let mut kind = kind();
        let k = Arc::make_mut(&mut kind);
        k.id = id.into();
        k.species = species;
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }

    /// A wolf's armor is the body slot, with the durability it lost; a
    /// helmet beside it reads from the head slot.
    #[test]
    fn a_wolfs_slots_read_apart() {
        let mut wolf = mob("minecraft:wolf", Species::Wolf);
        wolf.extra.set_body(Some("minecraft:wolf_armor".into()));
        wolf.extra.set_body_damage(9);
        wolf.extra.set_slot_id(
            horizon_items::equipment::HEAD,
            Some("minecraft:iron_helmet".into()),
        );
        let body = slot_item(&wolf, "body").unwrap();
        let head = slot_item(&wolf, "head").unwrap();
        assert_eq!(
            (body.item.as_str(), body.damage),
            ("minecraft:wolf_armor", 9)
        );
        assert_eq!(head.item, "minecraft:iron_helmet");
        // The armor was put on by `Equippable`, so it always drops.
        assert_eq!(drop_chance(&wolf, "body"), 2.0);
    }

    /// A body item nothing equipped (a `/item replace ... armor.body` on a
    /// zombie) drops by the default chance, as vanilla's unequipped slot.
    #[test]
    fn a_zombies_body_item_drops_by_the_default_chance() {
        let mut zombie = mob("minecraft:zombie", Species::Zombie);
        zombie.extra.set_body(Some("minecraft:iron_helmet".into()));
        assert_eq!(drop_chance(&zombie, "body"), DEFAULT_DROP_CHANCE);
        let body = slot_item(&zombie, "body").unwrap();
        assert_eq!(body.item, "minecraft:iron_helmet");
        assert!(slot_item(&zombie, "head").is_none());
    }

    /// `LivingEntity.dropExperience` and `Mob.dropCustomDeathLoot`: the
    /// memory timer (`playerKilled`) awards experience and equipment even
    /// when `LAST_DAMAGE_PLAYER` is gone (the player left).
    #[test]
    fn experience_and_equipment_follow_the_hurt_memory_not_the_player_entity() {
        let root =
            std::env::temp_dir().join(format!("horizon-guest-left-xp-{}", std::process::id()));
        std::fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let loot = LootData::load(&root, &PackOverlay::default(), &defaults).unwrap();
        let death = |player_killed| Death {
            loot: &loot,
            damage: None,
            player: None,
            player_killed,
            luck: 0.0,
            vehicle: None,
            mob_drops: true,
            world_seed: 0,
        };
        let zombie = |helmet: bool| {
            let mut m = mob("minecraft:zombie", Species::Zombie);
            if helmet {
                m.extra.set_slot_id(
                    horizon_items::equipment::HEAD,
                    Some("minecraft:iron_helmet".into()),
                );
                m.extra.drop_chances = Some(Box::new([1.0; 8]));
            }
            m
        };
        let orbs = |out: &[Drop]| {
            out.iter()
                .filter(|d| matches!(d, Drop::Experience(_)))
                .count()
        };
        let helmet = |out: &[Drop]| {
            out.iter().any(|d| match d {
                Drop::Item(s) => s.item == "minecraft:iron_helmet",
                Drop::Stack(s) => &*s.item == "minecraft:iron_helmet",
                _ => false,
            })
        };

        let mut m = zombie(false);
        let (out, _) = die(
            &mut m,
            &death(true),
            &mut RandomSequences::default(),
            &mut level_random(7),
        );
        assert!(orbs(&out) > 0, "memory timer awards experience: {out:?}");

        let mut m = zombie(false);
        let (out, _) = die(
            &mut m,
            &death(false),
            &mut RandomSequences::default(),
            &mut level_random(7),
        );
        assert_eq!(orbs(&out), 0, "no timer, no experience: {out:?}");

        let mut m = zombie(true);
        let (out, _) = die(
            &mut m,
            &death(true),
            &mut RandomSequences::default(),
            &mut level_random(7),
        );
        assert!(helmet(&out), "memory timer drops equipment: {out:?}");

        let mut m = zombie(true);
        let (out, _) = die(
            &mut m,
            &death(false),
            &mut RandomSequences::default(),
            &mut level_random(7),
        );
        assert!(!helmet(&out), "no timer, no equipment: {out:?}");
    }

    #[test]
    fn preserved_equipment_drops_when_chance_exceeds_one() {
        let mut m = mob("minecraft:zombie", Species::Zombie);
        m.extra.set_slot_stack(
            horizon_items::equipment::HEAD,
            Some(crate::items::ItemStack::new("minecraft:iron_helmet", 1)),
        );
        m.extra.drop_chances = Some(Box::new([2.0; 8]));
        let drops = drop_preserved_equipment(&mut m);
        assert_eq!(drops.len(), 1);
        assert_eq!(drops[0].item.as_ref(), "minecraft:iron_helmet");
        assert_eq!(m.extra.slot_stack(horizon_items::equipment::HEAD), None);
    }
}

#[cfg(test)]
#[path = "drops_tests.rs"]
mod drops_tests;
