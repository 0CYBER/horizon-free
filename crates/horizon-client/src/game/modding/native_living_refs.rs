//! Living entities for native server mods, through a `LivingRef`
//! (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387): the equipment
//! of mobs, players and armor stands, throwing a stack, and whether a mob
//! is tamed. Reads return `None`, writes false, for a stale handle or on a
//! client; the exception is the reads marked side `both` (`living`, `equipment`,
//! `hand_item`, `health`, `max_health`, `attribute_value`, `attribute_base`,
//! `attributes` and `attribute_modifiers`), which on a client answer for the
//! local player from the client's own copy; and `effect`/`effects`.
use super::Game;
use super::commands::{Target, items::Owner};
use super::living_attributes::AttributeWrite;
use crate::mobs::Who;
use horizon_client::items::ItemStack;
use horizon_client::mobs::Mob;
use horizon_world::attributes::{Modifier, Operation};
use mod_api::client::first_person::Hand;
use mod_api::{
    AttributeModifier, AttributeOperation, Direction, EntityRef, EquipmentSlot, Identifier,
    LivingRef, ModifierLifetime,
};

/// A player's armour slots (`armor`, head first) by equipment slot.
fn armor_index(slot: EquipmentSlot) -> Option<usize> {
    match slot {
        EquipmentSlot::Head => Some(0),
        EquipmentSlot::Chest => Some(1),
        EquipmentSlot::Legs => Some(2),
        EquipmentSlot::Feet => Some(3),
        _ => None,
    }
}

/// A player's stack in `slot`, from its inventory, armour and off hand.
fn player_slot(
    inventory: &[Option<ItemStack>],
    selected: usize,
    armor: &[Option<ItemStack>; 4],
    offhand: &Option<ItemStack>,
    equipment: &horizon_items::equipment::Equipment,
    slot: EquipmentSlot,
) -> Option<ItemStack> {
    match slot {
        EquipmentSlot::MainHand => inventory.get(selected).cloned().flatten(),
        EquipmentSlot::OffHand => offhand.clone(),
        EquipmentSlot::Body | EquipmentSlot::Saddle => equipment.get(slot as usize).cloned(),
        _ => armor[armor_index(slot)?].clone(),
    }
}

/// The equipment slot of a hand (`LivingEntity.getItemInHand`/`setItemInHand`).
fn hand_slot(hand: Hand) -> EquipmentSlot {
    match hand {
        Hand::Main => EquipmentSlot::MainHand,
        Hand::Off => EquipmentSlot::OffHand,
    }
}

/// The `/item` slot name of an equipment slot.
fn slot_name(slot: EquipmentSlot) -> &'static str {
    match slot {
        EquipmentSlot::MainHand => "weapon.mainhand",
        EquipmentSlot::OffHand => "weapon.offhand",
        EquipmentSlot::Head => "armor.head",
        EquipmentSlot::Chest => "armor.chest",
        EquipmentSlot::Legs => "armor.legs",
        EquipmentSlot::Feet => "armor.feet",
        EquipmentSlot::Body => "armor.body",
        EquipmentSlot::Saddle => "saddle",
    }
}

fn world_equipment_slot(slot: EquipmentSlot) -> horizon_core::equipment_slot::EquipmentSlot {
    use horizon_core::equipment_slot::EquipmentSlot as WorldSlot;
    match slot {
        EquipmentSlot::MainHand => WorldSlot::MainHand,
        EquipmentSlot::OffHand => WorldSlot::OffHand,
        EquipmentSlot::Head => WorldSlot::Head,
        EquipmentSlot::Chest => WorldSlot::Chest,
        EquipmentSlot::Legs => WorldSlot::Legs,
        EquipmentSlot::Feet => WorldSlot::Feet,
        EquipmentSlot::Body => WorldSlot::Body,
        EquipmentSlot::Saddle => WorldSlot::Saddle,
    }
}

/// `LivingEntity.isAlive`'s health test: above 0, so a NaN health loaded from
/// a save is not alive either.
fn alive_at(health: f32) -> bool {
    health > 0.0
}

fn equipment_stack(stack: Option<ItemStack>) -> Option<ItemStack> {
    stack.filter(|s| s.count > 0 && &*s.item != "minecraft:air")
}

/// The map's modifiers as the API's, in their order. Their ids are canonical
/// (`Modifier` normalizes them), so none is dropped.
fn attribute_modifiers(list: Vec<Modifier>) -> Vec<AttributeModifier> {
    list.iter().filter_map(api_modifier).collect()
}

/// One map modifier as the API's.
fn api_modifier(m: &Modifier) -> Option<AttributeModifier> {
    Some(AttributeModifier::new(
        Identifier::parse(m.id()).ok()?,
        m.amount(),
        match m.operation() {
            Operation::AddValue => AttributeOperation::AddValue,
            Operation::AddMultipliedBase => AttributeOperation::AddMultipliedBase,
            Operation::AddMultipliedTotal => AttributeOperation::AddMultipliedTotal,
        },
    ))
}

impl Game<'_> {
    /// What a living handle names now: a player, a mob, an armor stand or the dragon.
    pub(super) fn native_living_target(&self, living: LivingRef) -> Option<Target> {
        match self.native_entity_target(living.entity())? {
            t @ (Target::Player
            | Target::Guest(_)
            | Target::Mob(_)
            | Target::Stand(_)
            | Target::Dragon(_)) => Some(t),
            _ => None,
        }
    }
    /// The entity as a living entity (vanilla's `LivingEntity`: a mob, a
    /// player, an armor stand or the dragon), or None for another kind or a
    /// stale handle.
    ///
    /// On a client, only the local player is one: the client keeps the others'
    /// equipment, effects and health only as far as it draws them. API 382.
    pub(super) fn living(&self, entity: EntityRef) -> Option<LivingRef> {
        let living = LivingRef::new(entity);
        self.native_living_read_target(living).map(|_| living)
    }
    /// What a living handle names for a read that works on both sides: the
    /// server's entity, or on a client the local player (`Minecraft.player`),
    /// whose state is the client's copy of what the server sent.
    pub(super) fn native_living_read_target(&self, living: LivingRef) -> Option<Target> {
        if self.is_logical_server() {
            return self.native_living_target(living);
        }
        match self.native_client_entity_target(living.entity())? {
            Target::Player => Some(Target::Player),
            _ => None,
        }
    }
    /// The effective value of an attribute on a living entity
    /// (`AttributeMap.getValue`). None for an unknown or absent attribute, an
    /// unavailable entity, a stale handle, or on a client for any entity but
    /// the local player, whose copy of the synced instances it reads. A mob,
    /// an armor stand and the dragon answer from their live attribute map: the
    /// type's defaults, the saved `attributes`, and the modifiers of what they
    /// wear or hold.
    pub(super) fn living_attribute_value(&self, living: LivingRef, id: &str) -> Option<f64> {
        self.native_living_attribute(
            living,
            id,
            horizon_world::attribute_map::ValueKind::Computed,
        )
    }

    /// The base value of an attribute, before modifiers
    /// (`AttributeMap.getBaseValue`), read as the effective value is.
    pub(super) fn living_attribute_base(&self, living: LivingRef, id: &str) -> Option<f64> {
        self.native_living_attribute(living, id, horizon_world::attribute_map::ValueKind::Base)
    }

    /// The ids of the attributes the entity's type has
    /// (`AttributeMap`'s holder's attribute set), in registry order: the same
    /// set the value and base reads answer for. None for a stale handle or an
    /// unavailable entity; on a client, for any entity but the local player
    /// (the client keeps no attribute map of another entity's, only the few
    /// fields it draws).
    pub(super) fn living_attributes(&self, living: LivingRef) -> Option<Vec<Identifier>> {
        let catalog = &self.motion.attributes;
        let entity = self.native_living_attribute_entity(living)?;
        Some(
            catalog
                .attributes()
                .filter(|(id, _)| catalog.default_base(entity, *id).is_some())
                // The catalog's names are canonical, so none is dropped.
                .filter_map(|(_, a)| Identifier::parse(&a.name).ok())
                .collect(),
        )
    }

    /// The modifiers on the entity's attribute `id`
    /// (`AttributeInstance.getModifiers`), in the order they were added: a
    /// mod's transient and permanent ones, the saved list's, and what the
    /// worn and held items grant. An attribute no modifier is on answers an
    /// empty list; None for an attribute the entity type does not have, a
    /// stale handle, or on a client for any entity but the local player,
    /// whose copy of the synced instances it reads.
    pub(super) fn living_attribute_modifiers(
        &self,
        living: LivingRef,
        id: &str,
    ) -> Option<Vec<AttributeModifier>> {
        self.native_target_attribute_modifiers(self.native_living_read_target(living)?, id)
    }

    /// Adds an attribute modifier (`AttributeInstance.addTransientModifier`
    /// or `addPermanentModifier`). A permanent one is saved with the entity;
    /// a transient one lasts until it unloads. It goes where `/attribute`
    /// writes (`Game::write_attribute`), so gameplay and the entity's clients
    /// see it. False when the entity already has a modifier with that id on
    /// the attribute (vanilla throws), for an attribute the entity type does
    /// not have, a non-finite amount, the dragon (whose attributes nothing
    /// reads yet), a stale handle, or on a client.
    pub(super) fn living_add_attribute_modifier(
        &mut self,
        living: LivingRef,
        attribute: &str,
        modifier: AttributeModifier,
        lifetime: ModifierLifetime,
    ) -> bool {
        self.native_living_add_modifier(living, attribute, modifier, lifetime, false)
    }

    /// Adds an attribute modifier in place of the one with the same id, if
    /// any (`addOrUpdateTransientModifier`, `addOrReplacePermanentModifier`):
    /// the new one's lifetime wins. False as `add_attribute_modifier` is,
    /// except that a taken id is replaced.
    pub(super) fn living_replace_attribute_modifier(
        &mut self,
        living: LivingRef,
        attribute: &str,
        modifier: AttributeModifier,
        lifetime: ModifierLifetime,
    ) -> bool {
        self.native_living_add_modifier(living, attribute, modifier, lifetime, true)
    }

    /// Removes the modifier with id `id` from the attribute
    /// (`AttributeInstance.removeModifier`), whatever its lifetime. False when
    /// there was none, and in the cases `add_attribute_modifier` is.
    pub(super) fn living_remove_attribute_modifier(
        &mut self,
        living: LivingRef,
        attribute: &str,
        id: &Identifier,
    ) -> bool {
        self.native_living_write(living, attribute, AttributeWrite::Remove(id.to_string()))
    }

    /// Sets the attribute's base value (`AttributeInstance.setBaseValue`);
    /// the value stays within the attribute's range. False for a non-finite
    /// base, and in the cases `add_attribute_modifier` is.
    pub(super) fn living_set_attribute_base(
        &mut self,
        living: LivingRef,
        attribute: &str,
        base: f64,
    ) -> bool {
        base.is_finite() && self.native_living_write(living, attribute, AttributeWrite::Base(base))
    }

    fn native_living_add_modifier(
        &mut self,
        living: LivingRef,
        attribute: &str,
        modifier: AttributeModifier,
        lifetime: ModifierLifetime,
        replace: bool,
    ) -> bool {
        let operation = match modifier.operation {
            AttributeOperation::AddValue => Operation::AddValue,
            AttributeOperation::AddMultipliedBase => Operation::AddMultipliedBase,
            AttributeOperation::AddMultipliedTotal => Operation::AddMultipliedTotal,
            _ => unreachable!("AttributeOperation: unhandled variant"),
        };
        let permanent = match lifetime {
            ModifierLifetime::Transient => false,
            ModifierLifetime::Permanent => true,
            _ => unreachable!("ModifierLifetime: unhandled variant"),
        };
        if !modifier.amount.is_finite() {
            return false;
        }
        AttributeWrite::add(
            &modifier.id.to_string(),
            modifier.amount,
            operation,
            permanent,
            replace,
        )
        .is_some_and(|write| self.native_living_write(living, attribute, write))
    }

    fn native_living_write(
        &mut self,
        living: LivingRef,
        name: &str,
        write: AttributeWrite,
    ) -> bool {
        self.native_living_target(living)
            .and_then(|t| self.write_attribute(t, name, write))
            .unwrap_or(false)
    }

    fn native_living_attribute(
        &self,
        living: LivingRef,
        name: &str,
        kind: horizon_world::attribute_map::ValueKind,
    ) -> Option<f64> {
        self.native_target_attribute(self.native_living_read_target(living)?, name, kind)
    }

    /// The entity's type key, for the catalog's attribute defaults: the same
    /// set the value and base reads answer for. On a client only the local
    /// player resolves (`native_living_read_target`).
    fn native_living_attribute_entity(&self, living: LivingRef) -> Option<&str> {
        match self.native_living_read_target(living)? {
            Target::Player | Target::Guest(_) => Some("minecraft:player"),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)
                .map(|m| m.kind.id.as_str()),
            Target::Stand(_) => Some("minecraft:armor_stand"),
            Target::Dragon(_) => Some("minecraft:ender_dragon"),
            _ => None,
        }
    }

    /// The modifiers of `name` on `target`'s attribute map, as
    /// `native_target_attribute` reads its value.
    fn native_target_attribute_modifiers(
        &self,
        target: Target,
        name: &str,
    ) -> Option<Vec<AttributeModifier>> {
        let catalog = &self.motion.attributes;
        let player = match target {
            Target::Player => self.player.as_ref()?,
            Target::Guest(conn) => self.command_guest(conn)?.motion.as_deref()?,
            Target::Mob(id) => {
                let m = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
                return m
                    .extra
                    .live
                    .modifiers(catalog, &m.kind.id, name, || {
                        horizon_client::entity_nbt::attribute_entries(m)
                            .into_iter()
                            .map(|(_, entry)| entry)
                            .collect()
                    })
                    .map(attribute_modifiers);
            }
            Target::Stand(id) => {
                let a = self.armor_stands.iter().find(|a| a.id == id)?;
                return a
                    .live
                    .modifiers(catalog, "minecraft:armor_stand", name, Vec::new)
                    .map(attribute_modifiers);
            }
            Target::Dragon(id) => {
                let d = self.dragon.as_ref().filter(|d| d.id == id)?;
                return d
                    .live
                    .modifiers(catalog, "minecraft:ender_dragon", name, Vec::new)
                    .map(attribute_modifiers);
            }
            _ => return None,
        };
        // The base read's own None: an attribute the type does not have.
        let key = player.attributes.key(name)?;
        player
            .attributes
            .value(
                Some(key.clone()),
                horizon_world::attribute_map::ValueKind::Base,
            )
            .ok()?;
        Some(
            player
                .attributes
                .lookup(Some(&key))
                .and_then(|handle| player.attributes.instance(&handle).ok())
                .map(|instance| instance.modifiers())
                .unwrap_or_default()
                .iter()
                .filter_map(api_modifier)
                .collect(),
        )
    }

    pub(super) fn native_target_attribute(
        &self,
        target: Target,
        name: &str,
        kind: horizon_world::attribute_map::ValueKind,
    ) -> Option<f64> {
        let catalog = &self.motion.attributes;
        let player = match target {
            Target::Player => self.player.as_ref()?,
            Target::Guest(conn) => self.command_guest(conn)?.motion.as_deref()?,
            Target::Mob(id) => {
                let m = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
                return m.extra.live.read(catalog, &m.kind.id, name, kind, || {
                    horizon_client::entity_nbt::attribute_entries(m)
                        .into_iter()
                        .map(|(_, entry)| entry)
                        .collect()
                });
            }
            Target::Stand(id) => {
                let a = self.armor_stands.iter().find(|a| a.id == id)?;
                return a
                    .live
                    .read(catalog, "minecraft:armor_stand", name, kind, Vec::new);
            }
            Target::Dragon(id) => {
                let d = self.dragon.as_ref().filter(|d| d.id == id)?;
                return d
                    .live
                    .read(catalog, "minecraft:ender_dragon", name, kind, Vec::new);
            }
            _ => return None,
        };
        let key = player.attributes.key(name)?;
        player.attributes.value(Some(key), kind).ok()
    }

    /// `Entity.canUsePortal`: the decision the game's own portal trip runs —
    /// a player's (alive, nobody's passenger, not asleep: the engine's own
    /// `player_can_use_portal`, the one gate the client's portal entry, the
    /// guest trip and this read answer), a mob's walk gate
    /// (`mob_can_use_portal`, #1275's read: not a boss that never travels,
    /// not a creaking bound to its heart, not asleep in its bed, riding
    /// nothing, carrying no one this side holds back). None for a stale
    /// handle, an armor stand or the dragon: the portal walk carries
    /// neither as its own traveller.
    pub(super) fn living_can_use_portal(&self, living: LivingRef) -> Option<bool> {
        match self.native_living_read_target(living)? {
            // The engine's own gate, the one the client's portal entry runs
            // (`player_can_use_portal`, #943).
            Target::Player => Some(self.player_can_use_portal()),
            Target::Guest(conn) => {
                let g = self.command_guest(conn)?;
                let p = g.motion.as_deref()?;
                Some(self.can_use_portal_of(p, g.vehicle.is_some(), g.sleeping))
            }
            Target::Mob(id) => {
                let m = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
                Some(self.mob_portal_eligible(m))
            }
            _ => None,
        }
    }

    /// `Mob.isWithinMeleeAttackRange`: the melee goal's own reach check — the
    /// mob's box inflated by sqrt(2.04) − 0.6 over the target's box, or an
    /// active item's attack range for a mob (a spear's reach) — whether this
    /// living entity's next melee strike reaches `target`. The check is a
    /// mob's; a player's attack reach is another rule (the interaction range
    /// attribute), so a player answers None.
    pub(super) fn living_in_melee_range(
        &self,
        living: LivingRef,
        target: EntityRef,
    ) -> Option<bool> {
        let Target::Mob(id) = self.native_living_read_target(living)? else {
            return None;
        };
        let m = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
        let [min, max] = self.entity_bounding_box(target)?;
        let bounds = horizon_core::geometry::Aabb::new(
            horizon_core::geometry::Vec3::new(min[0], min[1], min[2]),
            horizon_core::geometry::Vec3::new(max[0], max[1], max[2]),
        );
        let w = self.mob_world_base(&[]);
        Some(m.melee_reaches(&w, bounds))
    }

    /// `LivingEntity.canFreeze`: the decision the freeze tick itself runs — a
    /// mob's (`ai::can_freeze`: not `#freeze_immune_entity_types`, no
    /// `#freeze_immune_wearables` worn), a player's (`PlayerMotion::can_freeze`:
    /// not a spectator, no immune armor). None for a stale handle, an armor
    /// stand or the dragon.
    pub(super) fn living_can_freeze(&self, living: LivingRef) -> Option<bool> {
        match self.native_living_read_target(living)? {
            Target::Mob(id) => {
                let m = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
                let w = self.mob_world_base(&[]);
                Some(horizon_client::mobs::ai::can_freeze(m, &w))
            }
            t => Some(self.native_player_body(t)?.can_freeze()),
        }
    }

    /// `LivingEntity.canBreatheUnderwater`: the species' own breath — a
    /// fish's, an axolotl's, a construct's (`breathes_water`, the
    /// `#can_breathe_under_water` tag); a player answers false (no player
    /// kind is in the tag). The tag read only, not the whole drown
    /// decision: the air tick itself also keeps the air up behind Water
    /// Breathing, the conduit's power and a player's invulnerability,
    /// which this read does not carry. None for a stale handle, an armor
    /// stand or the dragon.
    pub(super) fn living_can_breathe_underwater(&self, living: LivingRef) -> Option<bool> {
        match self.native_living_read_target(living)? {
            Target::Mob(id) => Some(
                self.mobs
                    .iter()
                    .find(|m| m.id == id && !m.removed)?
                    .kind
                    .flags
                    .breathes_water,
            ),
            Target::Player | Target::Guest(_) => Some(false),
            _ => None,
        }
    }

    /// The bed a sleeper lies in, as its block faces
    /// (`LivingEntity.getBedOrientation` over `BedBlock.getBedOrientation`):
    /// the block at the sleeping position, its `facing` where that block is
    /// one of `#minecraft:beds`. None where the sleeper keeps no position,
    /// the block is not a bed, or the handle is stale.
    pub(super) fn living_bed_orientation(&self, living: LivingRef) -> Option<Direction> {
        let pos = self.living_sleeping_pos(living)?;
        let w = self.mob_world_base(&[]);
        let name = w.block_name(pos[0], pos[1], pos[2]);
        if !w.has_tag("minecraft:beds", name) {
            return None;
        }
        match w.prop(pos[0], pos[1], pos[2], "facing") {
            Some("north") => Some(Direction::North),
            Some("south") => Some(Direction::South),
            Some("west") => Some(Direction::West),
            Some("east") => Some(Direction::East),
            _ => None,
        }
    }

    /// The entity's health (`LivingEntity.getHealth`; Paper's
    /// `Damageable#getHealth`). During a player's turn only that player
    /// resolves: the hub is out for their action, and other players'
    /// handles read `None` until it is back.
    pub(super) fn living_health(&self, living: LivingRef) -> Option<f32> {
        match self.native_living_read_target(living)? {
            Target::Player => Some(self.player.as_ref()?.health.health()),
            Target::Guest(conn) => Some(self.command_guest(conn)?.health),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)
                .map(|m| m.health),
            // An armor stand carries the fixed 20 the game gives it: lava
            // does not hurt it and splash water reads the very field.
            Target::Stand(id) => self
                .armor_stands
                .iter()
                .find(|a| a.id == id)
                .map(|a| a.health),
            Target::Dragon(id) => self
                .dragon
                .as_ref()
                .filter(|d| d.id == id)
                .map(|d| d.health),
            _ => None,
        }
    }
    /// The entity's maximum health (`LivingEntity.getMaxHealth`; Paper's
    /// `Damageable#getMaxHealth`). Players and guests read their attribute
    /// map; a mob, an armor stand and the dragon their own fields (the
    /// armor stand the fixed 20 the game seeds it with, the dragon its
    /// 200) until live attribute maps cover them.
    pub(super) fn living_max_health(&self, living: LivingRef) -> Option<f32> {
        let target = self.native_living_read_target(living)?;
        match target {
            Target::Player | Target::Guest(_) => self
                .native_target_attribute(
                    target,
                    "minecraft:max_health",
                    horizon_world::attribute_map::ValueKind::Computed,
                )
                .map(|max| max as f32),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)
                .map(|m| m.attrs.max_health),
            Target::Stand(_) => Some(20.0),
            Target::Dragon(_) => Some(super::dragon::MAX_HEALTH),
            _ => None,
        }
    }
    /// Whether the entity is alive (`LivingEntity.isAlive`; Paper's
    /// `Entity#isDead` is its inverse). A dying mob, mid death
    /// animation, is not.
    pub(super) fn living_alive(&self, living: LivingRef) -> Option<bool> {
        self.living_health(living).map(alive_at)
    }

    /// Sets the entity's health (`LivingEntity.setHealth`; Paper's
    /// `Damageable#setHealth`), cut to its maximum health. No heal or damage
    /// hook runs. False for a stale handle, a non-finite value, an armor stand
    /// or the dragon.
    pub(super) fn living_set_health(&mut self, living: LivingRef, health: f32) -> bool {
        if !health.is_finite() {
            return false;
        }
        match self.native_living_target(living) {
            Some(t @ (Target::Player | Target::Guest(_))) => self
                .for_player(t, |g| {
                    let max = g.player.as_mut()?.attribute("minecraft:max_health").ok()? as f32;
                    let health = health.min(max);
                    let revived = g.player.as_ref()?.health.health() <= 0.0 && health > 0.0;
                    g.journal_player_health();
                    g.player.as_mut()?.set_health(health, false).ok()?;
                    if revived
                        && let Some(conn) = g.lan_acting
                        && let Some(host) = g.lan.as_mut()
                        && let Some(gi) =
                            host.hub.guests.iter().position(|guest| guest.conn == conn)
                        && host.hub.guests[gi].death_canceled
                    {
                        host.hub.guests.update(gi, |guest| {
                            guest.death_canceled = false;
                            guest.death_time = 0;
                        });
                        g.player.as_mut()?.health.dead = false;
                    }
                    Some(())
                })
                .flatten()
                .is_some(),
            Some(Target::Mob(id)) => {
                self.journal_mob_health(id);
                let Some(m) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed) else {
                    return false;
                };
                let revived =
                    m.death_canceled && m.health <= 0.0 && health.min(m.attrs.max_health) > 0.0;
                m.health = health.clamp(0.0, m.attrs.max_health);
                m.needs_sync = true;
                if revived {
                    m.death_canceled = false;
                    m.death_time = 0;
                    self.died.remove(&id);
                }
                true
            }
            _ => false,
        }
    }
    /// Heals the entity by `amount` (`LivingEntity.heal`), through the
    /// `living_heal` hook, which may change or cancel it. A dead or dying
    /// entity isn't healed, and health stops at the maximum. False for a stale
    /// handle, a non-finite amount, a zero-health entity, an armor stand or the dragon.
    pub(super) fn living_heal(&mut self, living: LivingRef, amount: f32) -> bool {
        if !amount.is_finite()
            || !self
                .living_health(living)
                .is_some_and(|health| health > 0.0)
        {
            return false;
        }
        match self.native_living_target(living) {
            Some(t @ (Target::Player | Target::Guest(_))) => self
                .for_player(t, |g| {
                    g.apply_effect_action(horizon_client::effects::EffectAction::Heal(amount));
                })
                .is_some(),
            Some(Target::Mob(id)) => {
                let Some(m) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed) else {
                    return false;
                };
                m.heal(&self.loop_context, amount);
                m.needs_sync = true;
                true
            }
            _ => false,
        }
    }

    /// `LivingEntity.getKillCredit`: the remembered player takes precedence,
    /// even if the player cannot be resolved; otherwise the last hurting mob.
    /// Unlike vanilla, a mob that changed dimension cannot be resolved.
    pub(super) fn living_kill_credit(&self, living: LivingRef) -> Option<EntityRef> {
        let credit = match self.native_living_target(living)? {
            Target::Player => self.combat.player_credit(self.world.game_time),
            Target::Guest(conn) => self
                .guest_combat
                .get(&conn)
                .and_then(|combat| combat.player_credit(self.world.game_time)),
            Target::Mob(id) => self.last_hurt_by_player.get(&id).copied(),
            _ => None,
        };
        if let Some(credit) = credit {
            let conn = self.resolve_player_credit(credit)?;
            let id = self.player_entity_id_by_connection(conn)?;
            return self.entity(u32::try_from(id).ok()?);
        }
        self.living_last_attacker(living)
    }

    /// `LivingEntity.getLastHurtByMob`: the entity that last hurt this one,
    /// within a hundred ticks of the hurt (the window of
    /// `CombatTracker.getKiller`'s credit and a mob's hurt-by memory, as
    /// `armadillo.rs` reads it). None for a stale handle, a window that has
    /// passed, an attacker that is gone, an armor stand, the dragon or a
    /// client.
    pub(super) fn living_last_attacker(&self, living: LivingRef) -> Option<EntityRef> {
        match self.native_living_target(living)? {
            Target::Player => self
                .combat
                .kill_credit(self.world.game_time)
                .and_then(|id| self.entity(id)),
            Target::Guest(conn) => self
                .guest_combat
                .get(&conn)?
                .kill_credit(self.world.game_time)
                .and_then(|id| self.entity(id)),
            Target::Mob(id) => {
                let m = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
                let who =
                    m.ai.last_hurt_by
                        .filter(|_| m.age as i32 - m.ai.last_hurt_by_time <= 100)?;
                let attacker = match who {
                    Who::Mob(id) => Some(id),
                    // A player's hurt carries no id in `Who`: the hit's
                    // record names the player (`last_hurt_by_player`), the
                    // hub the guest's entity. The acting guest is resolved
                    // without the hub, which their turn takes out; the
                    // host's own record is displaced for that turn — the
                    // guest's body stands in as `player`, so the host's id
                    // is out of reach (the same None the other players'
                    // handles read while the hub is out).
                    Who::Player => {
                        let conn =
                            self.resolve_player_credit(*self.last_hurt_by_player.get(&id)?)?;
                        match conn {
                            None if self.lan_acting.is_some() => None,
                            None => Some(self.player.as_ref()?.entity_id as u32),
                            Some(conn) if self.lan_acting == Some(conn) => {
                                Some(self.lan_acting_entity?)
                            }
                            Some(conn) => Some(
                                self.lan
                                    .iter()
                                    .flat_map(|h| h.hub.guests.iter())
                                    .find(|g| g.conn == conn)?
                                    .entity_id as u32,
                            ),
                        }
                    }
                }?;
                self.entity(attacker)
            }
            _ => None,
        }
    }

    /// The stack in `slot` (`LivingEntity.getItemBySlot`). Players, guests,
    /// mobs, armor stands and the dragon store all eight slots. An armor
    /// stand's body and saddle are storage only: vanilla's `canUseSlot`
    /// refuses them for interaction. None means empty, unsupported or stale.
    /// The stack in equipment slot `slot` (`LivingEntity.getItemBySlot`):
    /// None when it is empty, the entity has no such slot (a player has no
    /// body or saddle slot), or the handle is stale. API 382.
    pub(super) fn living_equipment(
        &self,
        living: LivingRef,
        slot: EquipmentSlot,
    ) -> Option<ItemStack> {
        match self.native_living_read_target(living)? {
            Target::Player => player_slot(
                &self.inventory,
                self.selected,
                &self.armor,
                &self.offhand,
                &self.equipment,
                slot,
            ),
            Target::Guest(conn) => {
                let g = self.command_guest(conn)?;
                player_slot(
                    &g.inventory,
                    g.selected,
                    &g.armor,
                    &g.offhand,
                    &g.equipment,
                    slot,
                )
            }
            Target::Mob(id) => {
                self.mobs.iter().find(|m| m.id == id)?.equipment_stacks()[slot as usize].clone()
            }
            Target::Stand(id) => self
                .armor_stands
                .iter()
                .find(|a| a.id == id)?
                .equipment
                .get(slot as usize)
                .cloned(),
            Target::Dragon(id) => self
                .dragon
                .as_ref()
                .filter(|d| d.id == id)?
                .equipment
                .get(slot as usize)
                .cloned(),
            _ => None,
        }
    }
    /// Damage an equipped stack without an equip/unequip notification.
    pub(super) fn living_hurt_and_break(
        &mut self,
        living: LivingRef,
        slot: EquipmentSlot,
        amount: i32,
    ) -> Option<ItemStack> {
        let target = self.native_living_target(living)?;
        if matches!(target, Target::Player | Target::Guest(_)) {
            return self
                .for_player(target, |game| game.damage_player_equipment(slot, amount))
                .flatten();
        }
        let stack = self.living_equipment(living, slot)?;
        let wearer = super::durability::Wearer::Living(living.id());
        let (updated, broken) = self.damage_stack_worn(&stack, amount, wearer);
        if updated.as_ref() == Some(&stack) {
            return None;
        }
        if self.living_equipment(living, slot).as_ref() != Some(&stack) {
            return None;
        }
        match target {
            Target::Mob(id) => {
                self.mobs
                    .iter_mut()
                    .find(|mob| mob.id == id)?
                    .set_equipment_stack(slot as usize, updated);
            }
            Target::Stand(id) => {
                self.armor_stands
                    .iter_mut()
                    .find(|stand| stand.id == id)?
                    .equipment
                    .set(slot as usize, updated);
            }
            Target::Dragon(id) => {
                self.dragon
                    .as_mut()
                    .filter(|dragon| dragon.id == id)?
                    .equipment
                    .set(slot as usize, updated);
            }
            _ => return None,
        }
        if broken.is_some() {
            let event =
                horizon_client::mobs::item_break::equipment_break_event(world_equipment_slot(slot));
            self.entity_broadcast_event(living.entity(), event as u8);
        }
        broken
    }

    pub(super) fn damage_player_equipment(
        &mut self,
        slot: EquipmentSlot,
        amount: i32,
    ) -> Option<ItemStack> {
        let stack = player_slot(
            &self.inventory,
            self.selected,
            &self.armor,
            &self.offhand,
            &self.equipment,
            slot,
        )?;
        let (updated, broken) = self.damage_stack(&stack, amount, true);
        if updated.as_ref() == Some(&stack) {
            return None;
        }
        let cell = match slot {
            EquipmentSlot::MainHand => &mut self.inventory[self.selected],
            EquipmentSlot::OffHand => &mut self.offhand,
            EquipmentSlot::Body | EquipmentSlot::Saddle => {
                if self.equipment.get(slot as usize) != Some(&stack) {
                    return None;
                }
                self.equipment.set(slot as usize, updated);
                if let Some(ref broken) = broken {
                    self.award_stat("broken", &broken.item, 1);
                    self.player_equipment_broke(world_equipment_slot(slot), broken);
                }
                return broken;
            }
            _ => &mut self.armor[armor_index(slot)?],
        };
        if cell.as_ref() != Some(&stack) {
            return None;
        }
        *cell = updated;
        if let Some(ref broken) = broken {
            self.award_stat("broken", &broken.item, 1);
            self.player_equipment_broke(world_equipment_slot(slot), broken);
            if armor_index(slot).is_some() {
                self.apply_equipment();
            }
        }
        broken
    }

    /// The stack in hand `hand` (`LivingEntity.getItemInHand`). API 382.
    pub(super) fn living_hand_item(&self, living: LivingRef, hand: Hand) -> Option<ItemStack> {
        self.living_equipment(living, hand_slot(hand))
    }
    /// Puts `stack` in hand `hand` (`LivingEntity.setItemInHand`): the main
    /// hand's slot or the off hand's, written as `set_equipment`
    /// writes them, with its equip events and sound. None, air or a zero count
    /// empties the hand. False means unsupported or stale.
    pub(super) fn living_set_hand_item(
        &mut self,
        living: LivingRef,
        hand: Hand,
        stack: Option<ItemStack>,
    ) -> bool {
        self.living_set_equipment(living, hand_slot(hand), stack)
    }
    /// Puts `stack` in `slot` (`LivingEntity.setItemSlot`); None, air or a
    /// zero count empties it. Players, guests, mobs, armor stands and the
    /// dragon store all eight slots. An armor stand stores body and saddle
    /// too: `ArmorStand.canUseSlot` only stops interaction, and
    /// `LivingEntity.setItemSlot` keeps whatever it is given. False means
    /// unsupported or stale.
    /// This is a direct write: it takes effect and is final immediately, even
    /// inside `host.transaction`. A mob's write makes any equip/unequip event
    /// and equip sound immediately; a stand's or the dragon's makes neither. API 382.
    pub(super) fn living_set_equipment(
        &mut self,
        living: LivingRef,
        slot: EquipmentSlot,
        stack: Option<ItemStack>,
    ) -> bool {
        self.write_living_equipment(living, slot, stack, false)
    }
    /// The write `native_living_set_equipment` makes (#932): with `defer`,
    /// its equip sound and game event wait for the root commit of the open
    /// transaction — an abort stays quiet — while the equipment and the
    /// attribute map change at once, as later steps of the same transaction
    /// read them.
    pub(super) fn write_living_equipment(
        &mut self,
        living: LivingRef,
        slot: EquipmentSlot,
        stack: Option<ItemStack>,
        defer: bool,
    ) -> bool {
        // A client's copy changes only when the server says so.
        if self.remote.is_some() {
            return false;
        }
        let Some(t) = self.native_living_target(living) else {
            return false;
        };
        let stack = equipment_stack(stack);
        let equippable = stack.as_ref().and_then(|s| self.equippable_slot_sound(s));
        if let Target::Mob(id) = t {
            let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id) else {
                return false;
            };
            let old = if (slot as usize) <= 5 {
                horizon_client::mobs::finalize::item_stack(mob, slot as usize)
            } else {
                mob.equipment_stacks()[slot as usize].clone()
            };
            let changed = match (&old, &stack) {
                (None, None) => false,
                (Some(a), Some(b)) => !a.same_item_same_components(b),
                _ => true,
            };
            if !mob.set_equipment_stack(slot as usize, stack) {
                return false;
            }
            if changed && !mob.first_tick {
                let at = mob.position;
                let sound = equippable.as_ref().and_then(|(equip_slot, sound)| {
                    (!mob.silent && *equip_slot as usize == slot as usize)
                        .then(|| {
                            super::dispenser::equip_sound(mob, *equip_slot, Some(sound.clone()))
                        })
                        .flatten()
                });
                if defer {
                    self.after_commit(super::transactions::After::Equip {
                        at: [at.x, at.y, at.z],
                        sound,
                        equips: equippable.is_some(),
                        who: id,
                    });
                } else {
                    let ctx = self.who_context(Some(horizon_client::mobs::Who::Mob(id)), None);
                    self.equip_item_effects([at.x, at.y, at.z], sound, equippable.is_some(), ctx);
                }
            }
            // The stack's attribute modifiers go on the live map at once, so
            // the next read — a hit's armor, a `LivingRef`'s — sees them (#941).
            self.collect_mob_equipment_attributes(id);
            return true;
        }
        // `LivingEntity.onEquipItem` for the entities that keep no equip state
        // of their own: the stand and the dragon (not silent, never spectators).
        let (old, at, first_tick) = match t {
            Target::Stand(id) => self.armor_stands.iter().find(|a| a.id == id).map(|a| {
                (
                    a.equipment.get(slot as usize).cloned(),
                    a.position,
                    a.first_tick,
                )
            }),
            Target::Dragon(id) => self.dragon.as_ref().filter(|d| d.id == id).map(|d| {
                (
                    d.equipment.get(slot as usize).cloned(),
                    d.position,
                    d.first_tick(),
                )
            }),
            _ => None,
        }
        .map_or((None, None, true), |(old, at, first)| {
            (old, Some(at), first)
        });
        let written = match self
            .slots_named(&Owner::Entity(t), slot_name(slot))
            .as_slice()
        {
            // This path owns onEquipItem below, including immediate writes.
            // The generic stand slot writer must not emit it a second time.
            [s] => self.slot_set_with_effects(s, stack.clone(), false),
            _ => false,
        };
        if let (true, Some(at), false) = (written, at, first_tick) {
            let same = match (&old, &stack) {
                (None, None) => true,
                (Some(a), Some(b)) => a.same_item_same_components(b),
                _ => false,
            };
            if !same {
                let sound = equippable.as_ref().and_then(|(equip_slot, sound)| {
                    (*equip_slot as usize == slot as usize).then(|| sound.clone())
                });
                let (Target::Stand(id) | Target::Dragon(id)) = t else {
                    unreachable!("only a stand or the dragon gets here")
                };
                if defer {
                    self.after_commit(super::transactions::After::Equip {
                        at: [at.x, at.y, at.z],
                        sound,
                        equips: equippable.is_some(),
                        who: id,
                    });
                } else {
                    let ctx_who = Some(horizon_client::mobs::Who::Mob(id));
                    let ctx = self.who_context(ctx_who, None);
                    self.equip_item_effects([at.x, at.y, at.z], sound, equippable.is_some(), ctx);
                }
            }
        }
        written
    }

    /// The slot and sound `stack`'s `equippable` component names
    /// (`Equippable.slot`, `equipSound`).
    pub(super) fn equippable_slot_sound(
        &self,
        stack: &ItemStack,
    ) -> Option<(super::dispenser::Slot, String)> {
        let name = horizon_client::item_sync::equippable_slot(stack, self.items())?;
        let equip_slot = super::dispenser::Slot::from_name(&name)?;
        let sound = match stack.data().and_then(|d| d.other("minecraft:equippable")) {
            Some(component) => match component.get("equip_sound") {
                Some(horizon_core::nbt::Tag::String(sound)) => Some(sound.to_string_lossy()),
                _ => None,
            },
            None => self
                .items()
                .get(&stack.item)
                .and_then(|i| i.equip_sound.clone()),
        }
        .unwrap_or_else(|| "minecraft:item.armor.equip_generic".into());
        Some((equip_slot, sound))
    }

    /// The effects of `LivingEntity.onEquipItem` once the guards passed (server
    /// side, not a spectator, not the first tick, another item): the equip
    /// sound when there is one, and the `equip` (the stack is equippable) or
    /// `unequip` game event.
    pub(in crate::game) fn equip_item_effects(
        &mut self,
        at: [f64; 3],
        sound: Option<String>,
        equips: bool,
        ctx: horizon_client::sculk::Context,
    ) {
        if let Some(sound) = sound {
            self.audio.play(&sound, Some(at), 1.0, 1.0);
        }
        self.post_game_event(
            if equips {
                "minecraft:equip"
            } else {
                "minecraft:unequip"
            },
            at,
            ctx,
        );
    }
    /// Throws `stack`, which the mod holds, from the eyes (`LivingEntity.drop`,
    /// `ServerPlayer.drop`), never through a slot: along the view with a
    /// main-hand swing, or with `throw_randomly` scattered with none, as a
    /// death's drops. `include_thrower_name` sets the thrower, and a player's
    /// throw along the view then counts in its stats and fires `item_toss`.
    /// None for an empty stack, a stand or the dragon, a stale handle, or an
    /// item a mod kept out. API @API:living_refs@.
    pub(super) fn living_drop(
        &mut self,
        living: LivingRef,
        stack: ItemStack,
        throw_randomly: bool,
        include_thrower_name: bool,
    ) -> Option<EntityRef> {
        if self.entity_ids_exhausted() {
            return None;
        }
        let target = self.native_living_target(living)?;
        let stack = equipment_stack(Some(stack))?;
        let mut item = None;
        match target {
            Target::Mob(id) => {
                if !throw_randomly {
                    self.living_swing(living, Hand::Main, true);
                }
                item = self.native_mob_drop(id, stack, throw_randomly, include_thrower_name);
            }
            Target::Player | Target::Guest(_) => {
                self.native_as_living_player(target, |g| {
                    item = g.native_player_drop(stack, throw_randomly, include_thrower_name);
                });
            }
            _ => {}
        }
        Some(EntityRef::new(living.entity().epoch(), item?))
    }
    /// `TamableAnimal.isTame`, `AbstractHorse.isTamed`: None for a kind that
    /// can't be tamed or a stale handle. API @API:living_refs@.
    pub(super) fn living_is_tamed(&self, living: LivingRef) -> Option<bool> {
        self.native_tamable(living).map(|m| m.extra.tame)
    }
    /// The owner's UUID (`OwnableEntity.getOwnerReference`): None for a wild
    /// mob, a kind that can't be tamed, or a stale handle. API @API:living_refs@.
    pub(super) fn living_owner(&self, living: LivingRef) -> Option<u128> {
        let owner = self.native_tamable(living)?.extra.owner?;
        Some(horizon_client::lan::playerdata::ints_uuid(owner))
    }
    /// The mob a handle names, when it is a `TamableAnimal` or an `AbstractHorse`.
    fn native_tamable(&self, living: LivingRef) -> Option<&Mob> {
        let Target::Mob(id) = self.native_living_target(living)? else {
            return None;
        };
        self.mobs.iter().find(|m| m.id == id && m.is_tamable())
    }
    /// Runs `f` as the player `target` names: the one acting now, or a guest
    /// in the level in place through `as_guest` (`native_as_player`). False
    /// when it didn't run.
    fn native_as_living_player(&mut self, target: Target, f: impl FnOnce(&mut Self)) -> bool {
        let uuid = match target {
            Target::Guest(conn) => match self.command_guest(conn) {
                Some(g) if g.dimension == self.dim.id() => g.uuid,
                _ => return false,
            },
            _ => self.native_acting_id(),
        };
        self.native_as_player(uuid, f)
    }
    /// The acting player's half of `living_drop`: the id of the item entity.
    fn native_player_drop(
        &mut self,
        stack: ItemStack,
        randomly: bool,
        thrown_from_hand: bool,
    ) -> Option<u32> {
        if randomly {
            let eye = self.eye_position()?;
            self.throw_from_eyes(stack, eye, true, thrown_from_hand)
        } else if thrown_from_hand {
            self.player_drop(stack)
        } else {
            self.drop_plain(stack)
        }
    }
    /// A mob's half of `living_drop`: the id of the item entity.
    fn native_mob_drop(
        &mut self,
        id: u32,
        stack: ItemStack,
        randomly: bool,
        thrown_from_hand: bool,
    ) -> Option<u32> {
        let m = self.mobs.iter_mut().find(|m| m.id == id)?;
        let v = horizon_client::entities::drop_motion(&mut m.random, m.yaw, m.head_pitch, randomly);
        let at = [m.position.x, m.eye_y() - f64::from(0.3f32), m.position.z];
        let thrower = thrown_from_hand.then(|| self.uuid_for(id));
        self.spawn_item(stack, at, v, 40);
        let item = self.just_spawned_item()?;
        item.thrower = thrower;
        Some(item.id)
    }
}

#[cfg(test)]
mod tests {
    use super::super::api_values::TryIntoApi;
    use super::*;
    use horizon_core::geometry::Vec3;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn durability_handle_and_held_path_share_rng_and_damage() {
        use horizon_client::motion::PlayerMotion;
        use horizon_core::random::RandomSource;
        use mod_api::ServerHost;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let living = game.living(game.entity(20).unwrap()).unwrap();
            let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
            stack.data_mut().enchantments = vec![("minecraft:unbreaking".into(), 3)];
            game.inventory[game.selected] = Some(stack.clone());
            let random = game.world.random.clone();
            // Before the migration, damage_held called wear and added its result.
            let old_loss = game.wear(&stack, 20);
            let old_next_random = game.world.random.next_float();
            game.world.random = random.clone();
            game.damage_held(20);
            let held_result = game.inventory[game.selected].clone();
            let next_random = game.world.random.next_float();
            assert_eq!(held_result.as_ref().unwrap().damage, old_loss);
            assert_eq!(next_random, old_next_random);
            game.world.random = random;
            game.inventory[game.selected] = Some(stack);
            let host: &mut dyn ServerHost = game;
            assert!(
                living
                    .hurt_and_break(host, EquipmentSlot::MainHand, 20)
                    .is_none()
            );
            assert_eq!(game.inventory[game.selected], held_result);
            assert_eq!(game.world.random.next_float(), next_random);

            let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
            stack.damage = 10;
            game.inventory[game.selected] = Some(stack.clone());
            let mut repair_random = game.world.random.clone();
            assert!(
                living
                    .hurt_and_break(game, EquipmentSlot::MainHand, -4)
                    .is_none()
            );
            assert_eq!(game.world.random.next_float(), repair_random.next_float());
            assert_eq!(game.inventory[game.selected].as_ref().unwrap().damage, 6);
            game.creative = true;
            assert!(
                living
                    .hurt_and_break(game, EquipmentSlot::MainHand, 100)
                    .is_none()
            );
            assert_eq!(game.inventory[game.selected].as_ref().unwrap().damage, 6);
            game.creative = false;
            let mut unbreakable = stack.clone();
            unbreakable.data_mut().set_other(
                "minecraft:unbreakable",
                Some(horizon_core::nbt::Tag::Byte(1)),
            );
            game.inventory[game.selected] = Some(unbreakable.clone());
            assert!(
                living
                    .hurt_and_break(game, EquipmentSlot::MainHand, 100)
                    .is_none()
            );
            assert_eq!(game.inventory[game.selected], Some(unbreakable));
            let max = game.items().get(&stack.item).unwrap().max_damage;
            stack.damage = max - 1;
            game.inventory[game.selected] = Some(stack);
            let broken = living
                .hurt_and_break(game, EquipmentSlot::MainHand, 1)
                .unwrap();
            assert_eq!(broken.damage(), max);
            assert!(game.inventory[game.selected].is_none());

            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(21, kind, Vec3::ZERO, 0.0));
            let mob = game.living(game.entity(21).unwrap()).unwrap();
            let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
            sword.damage = max - 1;
            assert!(
                mob.set_equipment(
                    game,
                    EquipmentSlot::MainHand,
                    Some(sword.try_into_api().unwrap())
                )
                .unwrap()
            );
            let broken = mob
                .hurt_and_break(game, EquipmentSlot::MainHand, 1)
                .unwrap();
            assert_eq!(broken.damage(), max);
            assert!(mob.equipment(game, EquipmentSlot::MainHand).is_none());
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn armor_hit_matches_old_draws_and_break_order() {
        use horizon_client::motion::PlayerMotion;
        use horizon_core::random::RandomSource;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.audio.engine.loaded = true;
            game.audio.engine.registry.add_file("minecraft", br#"{"entity.item.break":{"sounds":["test/item"]},"block.anvil.break":{"sounds":["test/anvil"]}}"#, &|_, _| true);
            let source = horizon_client::health::NO_ATTACK.player_source("minecraft:generic", 0);
            let mut armor: [Option<ItemStack>; 4] = std::array::from_fn(|_| None);
            for (i, name) in ["minecraft:diamond_helmet", "minecraft:diamond_chestplate"]
                .into_iter()
                .enumerate()
            {
                let mut stack = ItemStack::new(name, 1);
                stack.damage = game.items().get(name).unwrap().max_damage - 1;
                let sound = ["minecraft:entity.item.break", "minecraft:block.anvil.break"][i];
                stack.data_mut().set_other(
                    "minecraft:break_sound",
                    Some(horizon_core::nbt::Tag::String(sound.into())),
                );
                armor[i] = Some(stack);
            }
            let mut leggings = ItemStack::new("minecraft:diamond_leggings", 1);
            leggings.data_mut().enchantments = vec![("minecraft:unbreaking".into(), 3)];
            armor[2] = Some(leggings);
            for guest in [false, true] {
                game.armor = armor.clone();
                game.lan_acting = guest.then_some(1);
                game.audio.capture = Some(Vec::new());
                game.audio.record_heard = true;
                game.audio.engine.notifications.clear();
                game.audio.heard.clear();
                game.guest_equipment_breaks.clear();
                let random = game.world.random.clone();
                // Player.hurtArmor supplies FEET, LEGS, CHEST, HEAD;
                // LivingEntity.doHurtEquipment finishes hurtAndBreak (including
                // its effects/RNG) before the next slot's durability draw.
                let mut broken = Vec::new();
                for index in [3, 2, 1, 0] {
                    let Some(mut stack) = game.armor[index].clone() else {
                        continue;
                    };
                    let max = game.items().get(&stack.item).unwrap().max_damage;
                    let loss = game.wear(&stack, 1);
                    if loss == 0 {
                        continue;
                    }
                    stack.damage = stack.damage.saturating_add(loss).min(max);
                    if stack.damage >= max {
                        game.armor[index] = None;
                        game.player_equipment_broke(super::super::armor_slot(index), &stack);
                        broken.push(index);
                    } else {
                        game.armor[index] = Some(stack);
                    }
                }
                let expected_armor = game.armor.clone();
                let expected_events = game.guest_equipment_breaks.clone();
                let expected_sounds = game.audio.heard.clone();
                let expected_sound_order: Vec<_> = game
                    .audio
                    .engine
                    .notifications
                    .iter()
                    .map(|event| event.identifier.clone())
                    .collect();
                let expected_random = game.world.random.next_float();
                assert_eq!(broken, [1, 0]);
                if guest {
                    assert_eq!(expected_events, [50, 49]);
                } else {
                    assert_eq!(expected_sounds.get("minecraft:entity.item.break"), Some(&1));
                    assert_eq!(expected_sounds.get("minecraft:block.anvil.break"), Some(&1));
                    assert_eq!(
                        expected_sound_order,
                        ["minecraft:block.anvil.break", "minecraft:entity.item.break"]
                    );
                }
                game.armor = armor.clone();
                game.guest_equipment_breaks.clear();
                game.audio.capture = Some(Vec::new());
                game.audio.heard.clear();
                game.audio.engine.notifications.clear();
                game.world.random = random;
                game.hurt_armor(&source, 4.0);
                assert_eq!(game.armor, expected_armor);
                assert_eq!(game.guest_equipment_breaks, expected_events);
                assert_eq!(game.audio.heard, expected_sounds);
                assert_eq!(
                    game.audio
                        .engine
                        .notifications
                        .iter()
                        .map(|event| event.identifier.clone())
                        .collect::<Vec<_>>(),
                    expected_sound_order
                );
                assert_eq!(game.world.random.next_float(), expected_random);
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn immutable_host_attribute_read_refreshes_live_cache() {
        use horizon_client::motion::PlayerMotion;
        use horizon_core::geometry::Vec3;
        use horizon_world::attributes::{Modifier, Operation};
        use mod_api::Host;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let living = game.living(game.entity(20).unwrap()).unwrap();
            let attributes = &mut game.player.as_mut().unwrap().attributes;
            let key = attributes.key("minecraft:max_health").unwrap();
            let instance = attributes.get_instance(Some(key.clone())).unwrap().unwrap();
            assert_eq!(attributes.instance(&instance).unwrap().value(), 20.0);
            attributes
                .edit(&instance, |a, o| {
                    a.add_transient(
                        Modifier::new("test:bonus", 5.0, Operation::AddValue).unwrap(),
                        o,
                    )
                })
                .unwrap();
            assert_eq!(attributes.cached_value(Some(&key)).unwrap(), 20.0);
            let host: &dyn Host = game;
            assert_eq!(
                living.attribute_base(host, "minecraft:max_health"),
                Some(20.0)
            );
            assert_eq!(
                game.player
                    .as_ref()
                    .unwrap()
                    .attributes
                    .cached_value(Some(&key))
                    .unwrap(),
                20.0
            );
            assert_eq!(
                living.attribute_value(host, "minecraft:max_health"),
                Some(25.0)
            );
            assert_eq!(living.max_health(host), Some(25.0));
            assert_eq!(living.attribute_value(host, "missing:attribute"), None);
            assert_eq!(
                game.player
                    .as_ref()
                    .unwrap()
                    .attributes
                    .cached_value(Some(&key))
                    .unwrap(),
                25.0
            );
        });
    }

    /// The reads a handle answers about an entity's attributes: the type's
    /// attribute ids, and one attribute's modifiers through the handle's own
    /// add, replace and remove writes.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_handle_lists_its_attributes_and_reads_its_modifiers() {
        use horizon_client::entities::ArmorStand;
        use horizon_client::mobs::Mob;
        use horizon_client::motion::PlayerMotion;
        use horizon_core::geometry::Vec3;
        use horizon_items::equipment::HEAD;
        use mod_api::{AttributeOperation, AttributeOperation::*, Identifier, ModifierLifetime::*};
        let modifier = |id: &str, amount: f64, operation: AttributeOperation| {
            AttributeModifier::new(Identifier::parse(id).unwrap(), amount, operation)
        };
        super::super::native_entity_refs::tests::with_game(|game| {
            let zombie_kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie_kind, Vec3::ZERO, 0.0));
            game.armor_stands.push(ArmorStand::new(13, Vec3::ZERO, 0.0));
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            let stand = game.living(game.entity(13).unwrap()).unwrap();

            // The zombie's type has these attributes, and the stand's lacks
            // attack damage.
            let listed = |living: mod_api::LivingRef| living.attributes(game).unwrap();
            for name in [
                "minecraft:max_health",
                "minecraft:movement_speed",
                "minecraft:attack_damage",
            ] {
                assert!(
                    listed(zombie).contains(&Identifier::parse(name).unwrap()),
                    "{name}"
                );
            }
            assert!(
                !listed(stand).contains(&Identifier::parse("minecraft:attack_damage").unwrap())
            );

            // An attribute with no modifier answers an empty list, one the
            // type lacks None.
            let speed = "minecraft:movement_speed";
            assert_eq!(zombie.attribute_modifiers(game, speed), Some(Vec::new()));
            assert_eq!(zombie.attribute_modifiers(game, "missing:attribute"), None);
            assert_eq!(
                stand.attribute_modifiers(game, "minecraft:attack_damage"),
                None
            );

            // The add, replace and remove writes the reads follow.
            assert!(zombie.add_attribute_modifier(
                game,
                speed,
                modifier("test:fast", 1.0, AddMultipliedBase),
                Permanent
            ));
            assert!(zombie.add_attribute_modifier(
                game,
                speed,
                modifier("test:boost", 0.5, AddMultipliedTotal),
                Transient
            ));
            assert_eq!(
                zombie.attribute_modifiers(game, speed),
                Some(vec![
                    modifier("test:fast", 1.0, AddMultipliedBase),
                    modifier("test:boost", 0.5, AddMultipliedTotal)
                ])
            );
            // Replacing puts the new one at the end, its lifetime changed.
            assert!(zombie.replace_attribute_modifier(
                game,
                speed,
                modifier("test:fast", 2.0, AddValue),
                Transient
            ));
            let boost = Identifier::parse("test:boost").unwrap();
            assert!(zombie.remove_attribute_modifier(game, speed, &boost));
            assert_eq!(
                zombie.attribute_modifiers(game, speed),
                Some(vec![modifier("test:fast", 2.0, AddValue)])
            );

            // A worn helmet's modifier is read off the stand's live map.
            game.armor_stands[0]
                .equipment
                .set(HEAD, Some(ItemStack::new("minecraft:diamond_helmet", 1)));
            game.tick_equipment_attributes();
            let armor = stand.attribute_modifiers(game, "minecraft:armor").unwrap();
            assert!(
                armor.iter().any(|m| (m.amount - 3.0).abs() < 1e-9),
                "{armor:?}"
            );

            // A player's own map answers the same read.
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.living(game.entity(20).unwrap()).unwrap();
            let health = "minecraft:max_health";
            assert_eq!(player.attribute_modifiers(game, health), Some(Vec::new()));
            assert!(player.add_attribute_modifier(
                game,
                health,
                modifier("test:hearty", 0.5, AddMultipliedBase),
                Permanent
            ));
            assert_eq!(
                player.attribute_modifiers(game, health),
                Some(vec![modifier("test:hearty", 0.5, AddMultipliedBase)])
            );

            // A stale handle reads None.
            game.mobs.clear();
            game.armor_stands.clear();
            assert_eq!(zombie.attributes(game), None);
            assert_eq!(zombie.attribute_modifiers(game, speed), None);
            assert_eq!(stand.attribute_modifiers(game, "minecraft:armor"), None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn living_reads_answer_the_vanilla_defaults_and_a_gone_entity_reads_none() {
        use horizon_client::entities::ArmorStand;
        use horizon_client::mobs::Mob;
        use horizon_client::motion::PlayerMotion;
        use horizon_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            game.armor_stands.push(ArmorStand::new(13, Vec3::ZERO, 0.0));
            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            // The eight dragon parts must not overlap the player at ID 20.
            dragon.id = 40;
            game.dragon.replace(dragon);
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            // The vanilla attribute defaults: a player and a zombie 20, an
            // armor stand the 20 the game seeds it with, the dragon its 200.
            for (id, health, max, alive) in [
                (20u32, 20.0, 20.0, true),
                (12, 20.0, 20.0, true),
                (13, 20.0, 20.0, true),
                (40, 200.0, 200.0, true),
            ] {
                let living = game.living(game.entity(id).unwrap()).unwrap();
                assert_eq!(living.health(game), Some(health), "health of {id}");
                assert_eq!(living.max_health(game), Some(max), "max of {id}");
                assert_eq!(living.alive(game), Some(alive), "alive of {id}");
            }
            // A dead dragon: isAlive is health above zero.
            let dragon_handle = game.living(game.entity(40).unwrap()).unwrap();
            game.dragon.as_mut().unwrap().health = 0.0;
            assert_eq!(dragon_handle.health(game), Some(0.0));
            assert_eq!(dragon_handle.alive(game), Some(false));
            // The entities gone: every read answers None.
            game.player = None;
            game.lan_acting_uuid = None;
            game.lan_acting_entity = None;
            game.mobs[0].removed = true;
            game.armor_stands.clear();
            game.dragon.discard();
            for id in [20u32, 12, 13, 40] {
                let handle = game.entity(id).and_then(|er| game.living(er));
                assert!(handle.is_none(), "handle of {id} after it is gone");
            }
            assert_eq!(dragon_handle.health(game), None);
            assert_eq!(dragon_handle.max_health(game), None);
            assert_eq!(dragon_handle.alive(game), None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn kill_credit_prefers_the_player_reference_and_never_falls_back_when_unresolved() {
        use horizon_client::mobs::{Damage, Mob};
        use horizon_core::geometry::Vec3;

        super::super::native_guest_turn_tests::with_guests(|game| {
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let wolf = game.res.mob_types().get("minecraft:wolf").unwrap().clone();
            game.mobs.push(Mob::new(100, pig, Vec3::ZERO, 0.0));
            let mut wolf = Mob::new(101, wolf, Vec3::ZERO, 0.0);
            wolf.extra.tame = true;
            wolf.extra.owner = Some(horizon_client::lan::playerdata::uuid_ints(12));
            game.mobs.push(wolf);
            game.mobs[0].ai.last_hurt_by = Some(Who::Mob(101));
            game.mobs[0].ai.last_hurt_by_time = game.mobs[0].age as i32;
            let living = game.living(game.entity(100).unwrap()).unwrap();
            assert_eq!(living.kill_credit(game).map(|e| e.id()), Some(101));

            // A player hit followed by a mob hit keeps the player credit.
            game.as_guest(12, |game| game.remember_mob_player_credit(100))
                .unwrap();
            assert_eq!(living.kill_credit(game).map(|e| e.id()), Some(12));
            // A tame wolf's hit also records its owner's reference.
            game.resolve_player_responsible(100, Damage::mob(101, [0.0; 3]));
            assert_eq!(living.kill_credit(game).map(|e| e.id()), Some(12));
            {
                let value = "minecraft:the_nether".into();
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.dimension = value);
            }
            assert_eq!(living.kill_credit(game).map(|e| e.id()), Some(12));
            {
                let value = false;
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.placed = value);
            }
            assert_eq!(living.kill_credit(game), None);

            // Once the player reference expires, the mob can be credited.
            game.last_hurt_by_player.remove(&100);
            assert_eq!(living.kill_credit(game).map(|e| e.id()), Some(101));
            // A mob that has left this level is no longer resolvable by its old ID.
            game.mobs[1].removed = true;
            assert_eq!(living.kill_credit(game), None);
            game.mobs[1].removed = false;
            game.mobs[0].age += 101;
            assert_eq!(living.kill_credit(game), None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_mobs_last_attacker_names_a_mob_the_host_and_none_when_displaced() {
        use horizon_client::mobs::Mob;
        use horizon_client::motion::PlayerMotion;
        use horizon_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(12, pig, Vec3::ZERO, 0.0));
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(15, zombie, Vec3::ZERO, 0.0));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.player.as_mut().unwrap().entity_id = 20;
            // The host's own turn: the uuid is acting, no guest connection.
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let pig_living = game.living(game.entity(12).unwrap()).unwrap();
            // A mob's hit names the mob's own id.
            game.mobs[0].ai.last_hurt_by = Some(Who::Mob(15));
            game.mobs[0].ai.last_hurt_by_time = game.mobs[0].age as i32;
            assert_eq!(pig_living.last_attacker(game).map(|a| a.id()), Some(15));
            // The host's hit, read in the host's own turn: the host's handle.
            game.mobs[0].ai.last_hurt_by = Some(Who::Player);
            game.last_hurt_by_player
                .insert(12, super::super::combat::PlayerCredit(123));
            assert_eq!(pig_living.last_attacker(game).map(|a| a.id()), Some(20));
            // The same record inside a guest's turn: the host's displaced
            // body is out of reach, so the attacker reads None.
            game.lan_acting = Some(7);
            game.lan_acting_uuid = Some(456);
            assert_eq!(pig_living.last_attacker(game), None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn set_health_can_store_zero() {
        use horizon_client::mobs::Mob;
        use horizon_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            let living = game.living(game.entity(12).unwrap()).unwrap();
            assert!(living.set_health(game, 8.0));
            assert_eq!(living.health(game), Some(8.0));
            let accepted = living.set_health(game, 0.0);
            assert_eq!((accepted, living.health(game)), (true, Some(0.0)));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn set_health_journal_restores_nested_player_and_mob_writes() {
        use horizon_client::motion::PlayerMotion;
        use horizon_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            game.mobs[0].needs_sync = false;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.player.as_mut().unwrap().entity_id = 20;
            super::super::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.living(game.entity(20).unwrap()).unwrap();
            let mob = game.living(game.entity(12).unwrap()).unwrap();
            let initial = (player.health(game), mob.health(game));
            let dirty = game.player.as_ref().unwrap().health.data.is_dirty();
            let events = game.player.as_ref().unwrap().events.len();
            let outer = game.transaction_open();
            assert!(player.set_health(game, 12.0));
            assert!(mob.set_health(game, 8.0));
            let inner = game.transaction_open();
            assert!(player.set_health(game, 0.0));
            assert!(mob.set_health(game, 0.0));
            game.transaction_close(&inner, false);
            assert_eq!(
                (player.health(game), mob.health(game)),
                (Some(12.0), Some(8.0))
            );
            let inner = game.transaction_open();
            assert!(player.set_health(game, 0.0));
            assert!(mob.set_health(game, 0.0));
            game.transaction_close(&inner, true);
            game.transaction_close(&outer, false);
            assert_eq!((player.health(game), mob.health(game)), initial);
            assert_eq!(game.player.as_ref().unwrap().health.data.is_dirty(), dirty);
            assert_eq!(game.player.as_ref().unwrap().events.len(), events);
            assert!(!game.mobs[0].needs_sync);
            assert!(!game.player.as_ref().unwrap().health.dead);
            assert!(!game.mobs[0].removed);
            game.mobs[0].needs_sync = true;
            let abort = game.transaction_open();
            assert!(mob.set_health(game, 0.0));
            game.transaction_close(&abort, false);
            assert_eq!(mob.health(game), initial.1);
            assert!(
                game.mobs[0].needs_sync,
                "abort preserves preexisting sync dirtiness"
            );
            let commit = game.transaction_open();
            assert!(player.set_health(game, 0.0));
            assert!(mob.set_health(game, 0.0));
            game.transaction_close(&commit, true);
            assert_eq!(
                (player.health(game), mob.health(game)),
                (Some(0.0), Some(0.0))
            );
            assert!(game.mobs[0].needs_sync);
            assert!(game.player.as_ref().unwrap().events.len() > events);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn set_health_journal_restores_zero_health_guest_outside_its_turn() {
        super::super::native_guest_turn_tests::with_guests(|game| {
            let guest = game.player(12).unwrap();
            let living = LivingRef::new(guest.entity());
            let before = living.health(game).unwrap();
            let tx = game.transaction_open();
            assert!(living.set_health(game, before - 2.0));
            let inner = game.transaction_open();
            assert!(living.set_health(game, 0.0));
            assert_eq!(living.health(game), Some(0.0));
            game.transaction_close(&inner, false);
            assert_eq!(living.health(game), Some(before - 2.0));
            let inner = game.transaction_open();
            assert!(living.set_health(game, 0.0));
            game.transaction_close(&inner, true);
            game.transaction_close(&tx, false);
            assert_eq!(living.health(game), Some(before));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn heal_uses_the_vanilla_signed_sum() {
        use horizon_client::mobs::Mob;
        use horizon_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            let living = game.living(game.entity(12).unwrap()).unwrap();
            assert!(living.set_health(game, 10.0));
            assert!(living.heal(game, 2.0));
            assert_eq!(living.health(game), Some(12.0));
            let _ = living.heal(game, -4.0);
            assert_eq!(living.health(game), Some(8.0));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn absorbed_player_hit_still_remembers_the_living_attacker() {
        use horizon_client::mobs::Mob;
        use horizon_client::motion::PlayerMotion;
        use horizon_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(15, zombie, Vec3::ZERO, 0.0));
            let source = game
                .motion
                .health
                .damage
                .id("minecraft:mob_attack")
                .unwrap();
            let mut attackers = Vec::new();
            let mut health = Vec::new();
            for absorption in [0.0, 8.0] {
                game.combat = Default::default();
                let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
                player.entity_id = 20;
                player
                    .set_modifier(
                        "minecraft:max_absorption",
                        "test:absorption",
                        8.0,
                        "add_value",
                        true,
                    )
                    .unwrap();
                player.set_health(absorption, true).unwrap();
                game.player = Some(player);
                crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
                game.note_attacker("minecraft:mob_attack", 15);
                let rules = game.world.health_rules;
                let level = super::super::entity::damage_level_of!(game);
                let player = game.player.as_mut().unwrap();
                assert!(
                    player
                        .hurt_caused_by(
                            level,
                            &game.motion.health,
                            rules,
                            source,
                            4.0,
                            None,
                            true,
                            15,
                            false
                        )
                        .unwrap()
                        .hurt
                );
                let events = std::mem::take(&mut player.events);
                game.record_combat(&events);
                let living = game.living(game.entity(20).unwrap()).unwrap();
                health.push(living.health(game));
                let host: &dyn mod_api::Host = game;
                attackers.push(living.last_attacker(host).map(|attacker| attacker.id()));
            }
            assert_eq!(
                health,
                vec![Some(16.0), Some(20.0)],
                "damage/absorption control"
            );
            assert_eq!(attackers, vec![Some(15), Some(15)]);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn raw_health_write_can_change_a_dead_entity_but_heal_cannot() {
        use horizon_client::mobs::Mob;
        use horizon_client::motion::PlayerMotion;
        use horizon_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(12, pig, Vec3::ZERO, 0.0));
            // Dying, mid death animation: not removed, but not alive.
            game.mobs[0].health = 0.0;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.player.as_mut().unwrap().entity_id = 20;
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let mob_living = game.living(game.entity(12).unwrap()).unwrap();
            assert!(!mob_living.heal(game, 10.0));
            assert_eq!(mob_living.health(game), Some(0.0));
            assert!(mob_living.set_health(game, 10.0));
            assert_eq!(mob_living.health(game), Some(10.0));
            // A raw write also reaches a zero-health player.
            game.player
                .as_mut()
                .unwrap()
                .set_health(0.0, false)
                .unwrap();
            let player_living = game.living(game.entity(20).unwrap()).unwrap();
            assert!(!player_living.heal(game, 10.0));
            assert_eq!(player_living.health(game), Some(0.0));
            assert!(player_living.set_health(game, 10.0));
            assert_eq!(player_living.health(game), Some(10.0));
        });
    }

    /// `set_hand_item` writes the hand's equipment slot, as vanilla's
    /// `setItemInHand` does: a player's selected slot or off hand, a mob's
    /// main or off hand; a stale handle writes nothing.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn set_hand_item_writes_the_hands_slot() {
        use horizon_client::mobs::Mob;
        use horizon_client::motion::PlayerMotion;
        use mod_api::ServerHost;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            game.selected = 3;
            let player = game.living(game.entity(20).unwrap()).unwrap();
            let sword = ItemStack::new("minecraft:iron_sword", 1);
            let shield = ItemStack::new("minecraft:shield", 1);
            let host: &mut dyn ServerHost = game;
            assert!(
                player
                    .set_hand_item(host, Hand::Main, Some(sword.try_into_api().unwrap()))
                    .unwrap()
            );
            assert!(
                player
                    .set_hand_item(host, Hand::Off, Some(shield.try_into_api().unwrap()))
                    .unwrap()
            );
            assert_eq!(game.inventory[3], Some(sword.clone()));
            assert_eq!(game.offhand, Some(shield.clone()));
            assert_eq!(
                player.hand_item(game, Hand::Main),
                Some(sword.try_into_api().unwrap())
            );
            assert_eq!(
                player.hand_item(game, Hand::Off),
                Some(shield.try_into_api().unwrap())
            );
            assert!(player.set_hand_item(game, Hand::Off, None).unwrap());
            assert_eq!(game.offhand, None);
            assert_eq!(game.inventory[3], Some(sword.clone()));

            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, kind, Vec3::ZERO, 0.0));
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            assert!(
                zombie
                    .set_hand_item(game, Hand::Off, Some(sword.try_into_api().unwrap()))
                    .unwrap()
            );
            let stacks = game.mobs[0].equipment_stacks();
            assert_eq!(stacks[EquipmentSlot::OffHand as usize], Some(sword.clone()));
            assert_eq!(stacks[EquipmentSlot::MainHand as usize], None);

            game.mobs.clear();
            assert!(
                !zombie
                    .set_hand_item(game, Hand::Main, Some(sword.try_into_api().unwrap()))
                    .unwrap()
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn mob_equipment_writes_emit_vanilla_equip_events() {
        use horizon_client::mobs::Mob;
        use horizon_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, kind, Vec3::ZERO, 0.0));
            let living = game.living(game.entity(12).unwrap()).unwrap();
            game.audio.capture = Some(Vec::new());
            super::super::dispense_fixture::start_recording();
            let helmet = ItemStack::new("minecraft:iron_helmet", 1);
            assert!(game.living_set_equipment(living, EquipmentSlot::Head, Some(helmet.clone())));
            assert!(game.living_set_equipment(living, EquipmentSlot::Head, None));
            assert!(game.audio.capture.as_ref().unwrap().is_empty());
            game.mobs[0].first_tick = false;
            assert!(game.living_set_equipment(living, EquipmentSlot::Head, Some(helmet.clone())));
            assert_eq!(game.audio.capture.as_ref().unwrap().len(), 1);
            assert!(game.living_set_equipment(living, EquipmentSlot::Head, Some(helmet)));
            assert_eq!(game.audio.capture.as_ref().unwrap().len(), 1);
            game.mobs[0].silent = true;
            assert!(game.living_set_equipment(
                living,
                EquipmentSlot::Head,
                Some(ItemStack::new("minecraft:diamond_helmet", 1))
            ));
            assert!(game.living_set_equipment(living, EquipmentSlot::Head, None));
            game.mobs[0].silent = false;
            // Equippable, but its head slot does not match chest: event, no sound.
            assert!(game.living_set_equipment(
                living,
                EquipmentSlot::Chest,
                Some(ItemStack::new("minecraft:iron_helmet", 1))
            ));
            assert_eq!(game.audio.capture.as_ref().unwrap().len(), 1);
            let events = super::super::dispense_fixture::stop_recording();
            assert_eq!(
                events
                    .iter()
                    .filter(|e| e.ends_with("minecraft:equip"))
                    .count(),
                3
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|e| e.ends_with("minecraft:unequip"))
                    .count(),
                1
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn stand_and_dragon_equipment_writes_run_on_equip_item() {
        use horizon_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let mut stand = horizon_client::entities::ArmorStand::new(16, Vec3::ZERO, 0.0);
            stand.id = 16;
            game.armor_stands.push(stand);
            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 17;
            game.dragon.replace(dragon);
            let stand = game.living(game.entity(16).unwrap()).unwrap();
            let dragon = game.living(game.entity(17).unwrap()).unwrap();
            game.audio.capture = Some(Vec::new());
            super::super::dispense_fixture::start_recording();
            let helmet = ItemStack::new("minecraft:iron_helmet", 1);
            // Before its first tick an entity makes no sound and no event.
            assert!(game.living_set_equipment(stand, EquipmentSlot::Head, Some(helmet.clone())));
            assert!(game.living_set_equipment(stand, EquipmentSlot::Head, None));
            assert!(game.audio.capture.as_ref().unwrap().is_empty());
            game.tick_armor_stands().unwrap();
            game.dragon.as_mut().unwrap().ticks = 1;
            // An equippable item in its own slot: sound and `equip`; the same
            // item again changes nothing.
            for _ in 0..2 {
                assert!(game.living_set_equipment(
                    stand,
                    EquipmentSlot::Head,
                    Some(helmet.clone())
                ));
            }
            // A stored body armor: sound and `equip`, body is storage only.
            let body = ItemStack::new("minecraft:diamond_horse_armor", 1);
            assert!(game.living_set_equipment(stand, EquipmentSlot::Body, Some(body)));
            // Not equippable: `unequip` and no sound.
            let stone = ItemStack::new("minecraft:stone", 1);
            assert!(game.living_set_equipment(stand, EquipmentSlot::Saddle, Some(stone.clone())));
            // The dragon the same.
            let chest = ItemStack::new("minecraft:iron_chestplate", 1);
            assert!(game.living_set_equipment(dragon, EquipmentSlot::Chest, Some(chest)));
            assert!(game.living_set_equipment(dragon, EquipmentSlot::Saddle, Some(stone)));
            assert_eq!(game.audio.capture.as_ref().unwrap().len(), 3);
            let events = super::super::dispense_fixture::stop_recording();
            let count = |name: &str| events.iter().filter(|e| e.ends_with(name)).count();
            assert_eq!(count("minecraft:equip"), 3);
            assert_eq!(count("minecraft:unequip"), 2);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn mob_equipment_writes_keep_full_stacks() {
        use horizon_client::mobs::Mob;
        use horizon_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            for (id, kind) in [
                (12, "minecraft:zombie"),
                (13, "minecraft:horse"),
                (14, "minecraft:pillager"),
            ] {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
            }
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            let horse = game.living(game.entity(13).unwrap()).unwrap();
            let captain = game.living(game.entity(14).unwrap()).unwrap();
            let mut helmet = ItemStack::new("minecraft:iron_helmet", 2);
            helmet.damage = 7;
            helmet.data_mut().enchantments = vec![("minecraft:protection".into(), 3)];
            assert_eq!(
                zombie.set_equipment(
                    game,
                    EquipmentSlot::Head,
                    Some(helmet.try_into_api().unwrap())
                ),
                Err(mod_api::StackWriteError::CountExceeded { maximum: 1 })
            );
            assert!(game.living_set_equipment(zombie, EquipmentSlot::Head, Some(helmet.clone())));
            assert_eq!(
                zombie.equipment(game, EquipmentSlot::Head),
                Some(helmet.try_into_api().unwrap())
            );
            assert_eq!(
                game.mobs[0]
                    .extra
                    .slot_stack(horizon_items::equipment::HEAD),
                Some(&helmet)
            );
            let mut shield = ItemStack::new("minecraft:shield", 2);
            shield.damage = 11;
            assert!(game.living_set_equipment(
                zombie,
                EquipmentSlot::OffHand,
                Some(shield.clone())
            ));
            assert_eq!(
                game.living_equipment(zombie, EquipmentSlot::OffHand),
                Some(shield)
            );
            assert!(game.living_set_equipment(horse, EquipmentSlot::Head, Some(helmet.clone())));
            let mut body = ItemStack::new("minecraft:diamond_horse_armor", 3);
            body.damage = 9;
            body.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("armor".into()));
            assert!(game.living_set_equipment(horse, EquipmentSlot::Body, Some(body.clone())));
            assert_eq!(
                game.living_equipment(horse, EquipmentSlot::Head),
                Some(helmet.clone())
            );
            assert_eq!(
                game.living_equipment(horse, EquipmentSlot::Body),
                Some(body)
            );
            for slot in [
                EquipmentSlot::MainHand,
                EquipmentSlot::Chest,
                EquipmentSlot::Legs,
                EquipmentSlot::Feet,
            ] {
                let stack = ItemStack::new("minecraft:stone", 4);
                assert!(game.living_set_equipment(zombie, slot, Some(stack.clone())));
                assert_eq!(game.living_equipment(zombie, slot), Some(stack));
                assert!(game.living_set_equipment(
                    zombie,
                    slot,
                    Some(ItemStack::new("minecraft:stone", 0))
                ));
                assert_eq!(game.living_equipment(zombie, slot), None);
            }
            let mut saddle = ItemStack::new("minecraft:saddle", 3);
            saddle.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("saddle".into()));
            assert!(game.living_set_equipment(horse, EquipmentSlot::Saddle, Some(saddle.clone())));
            assert_eq!(
                game.living_equipment(horse, EquipmentSlot::Saddle),
                Some(saddle)
            );
            horizon_client::mobs::raider::set_patrol_leader(&mut game.mobs[2], true);
            horizon_client::mobs::raider::wear_banner(&mut game.mobs[2]);
            assert!(horizon_client::mobs::raider::is_captain(&game.mobs[2]));
            assert!(game.living_set_equipment(captain, EquipmentSlot::Head, Some(helmet.clone())));
            assert_eq!(
                game.living_equipment(captain, EquipmentSlot::Head),
                Some(helmet)
            );
            assert!(!horizon_client::mobs::raider::is_captain(&game.mobs[2]));
            assert!(game.living_set_equipment(
                horse,
                EquipmentSlot::Body,
                Some(ItemStack::new("minecraft:air", 1))
            ));
            assert_eq!(game.living_equipment(horse, EquipmentSlot::Body), None);
            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            // Reserve the dragon's eight consecutive part IDs away from the stand.
            dragon.id = 40;
            game.dragon.replace(dragon);
            let dragon = game.living(game.entity(40).unwrap()).unwrap();
            // The dragon and an armor stand store all eight slots, full
            // stacks included, and save them through the real codecs.
            let mut stand = horizon_client::entities::ArmorStand::new(16, Vec3::ZERO, 0.0);
            stand.id = 16;
            game.armor_stands.push(stand);
            let stand = game.living(game.entity(16).unwrap()).unwrap();
            for living in [dragon, stand] {
                for slot in EquipmentSlot::ALL {
                    let mut stack = ItemStack::new("minecraft:stone", 3);
                    stack.damage = 5 + slot as u32;
                    stack.data_mut().custom_name =
                        Some(horizon_core::nbt::Tag::String(slot_name(slot).into()));
                    assert!(game.living_set_equipment(living, slot, Some(stack.clone())));
                    assert_eq!(game.living_equipment(living, slot), Some(stack));
                }
            }
            let saved = game.dragon.as_ref().unwrap().to_nbt([0; 4], None);
            let loaded = super::super::dragon::Dragon::from_nbt(&saved, 0).unwrap();
            assert_eq!(loaded.equipment, game.dragon.as_ref().unwrap().equipment);
            let stored = game.armor_stands.last().unwrap();
            let saved = horizon_client::entity_nbt::armor_stand_to_nbt(stored, [0; 4], None);
            let loaded = horizon_client::entity_nbt::armor_stand_from_nbt(&saved, 16).unwrap();
            assert_eq!(loaded.equipment, stored.equipment);
            for living in [dragon, stand] {
                assert!(game.living_set_equipment(living, EquipmentSlot::Saddle, None));
                assert_eq!(game.living_equipment(living, EquipmentSlot::Saddle), None);
            }
            // An empty dragon saves no `equipment` key, a stale one included.
            for slot in EquipmentSlot::ALL {
                assert!(game.living_set_equipment(dragon, slot, None));
            }
            let stale = game.dragon.as_ref().unwrap().to_nbt([0; 4], Some(&saved));
            assert!(stale.get("equipment").is_none());
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_host_players_body_and_saddle_are_slots_and_saved_in_equipment() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let owner = Owner::Entity(Target::Player);
            let mut body = ItemStack::new("minecraft:diamond_horse_armor", 1);
            body.damage = 4;
            body.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("armor".into()));
            let saddle = ItemStack::new("minecraft:saddle", 1);
            game.player = Some(
                horizon_client::motion::PlayerMotion::new(
                    game.motion,
                    horizon_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.audio.capture = Some(Vec::new());
            for (name, stack) in [("armor.body", &body), ("saddle", &saddle)] {
                let slots = game.slots_named(&owner, name);
                assert_eq!(slots.len(), 1, "{name} is one slot");
                assert!(game.slot_set(&slots[0], Some(stack.clone())));
            }
            assert_eq!(
                (game.equipment.get(6), game.equipment.get(7)),
                (Some(&body), Some(&saddle))
            );
            // onEquipItem: each item in its own slot plays its equip sound.
            assert_eq!(game.audio.capture.as_ref().unwrap().len(), 2);
            game.sync_level();
            let tag = game.level.player_tag();
            let equipment = tag.get("equipment").unwrap();
            assert!(equipment.get("body").is_some() && equipment.get("saddle").is_some());
            let mut read = horizon_client::save::Level::default();
            read.read_player(&tag);
            // Loading the file back into a Game restores both, components too.
            game.equipment = Default::default();
            let level = read;
            game.load_saved_inventory(&level);
            assert_eq!(
                (game.equipment.get(6), game.equipment.get(7)),
                (Some(&body), Some(&saddle))
            );
        });
    }

    #[test]
    fn alive_is_health_above_zero_as_in_vanilla() {
        assert!(alive_at(0.5));
        assert!(!alive_at(0.0));
        assert!(!alive_at(-1.0));
        assert!(!alive_at(f32::NAN));
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn armor_stand_first_tick_equipment_change_fires_after_base_tick() {
        use mod_api::{self, Hooks, LivingEquipmentChangeEvent, ModEntry, ServerHost};
        struct Reequip;
        #[mod_api::__subscriptions]
        impl Hooks for Reequip {
            fn living_equipment_change(
                &self,
                host: &mut dyn ServerHost,
                event: &mut LivingEquipmentChangeEvent<'_>,
            ) {
                if event.entity == 18 && event.slot == "mainhand" {
                    let entity = host.entity(event.entity).unwrap();
                    let living = host.living(entity).unwrap();
                    assert!(
                        living
                            .set_equipment(
                                host,
                                EquipmentSlot::Head,
                                Some(
                                    ItemStack::new("minecraft:iron_helmet", 1)
                                        .try_into_api()
                                        .unwrap()
                                ),
                            )
                            .unwrap()
                    );
                }
            }
        }
        mod_platform::testing::with_mods(
            vec![ModEntry::new("first_tick_equipment", Box::new(Reequip))].into(),
            || {
                super::super::native_entity_refs::tests::with_game(|game| {
                    let mut stand = horizon_client::entities::ArmorStand::new(
                        18,
                        Vec3::new(0.5, 70.0, 0.5),
                        0.0,
                    );
                    stand.no_gravity = true;
                    stand.equipment[0] = Some(ItemStack::new("minecraft:stone", 1));
                    game.armor_stands.push(stand);
                    game.audio.capture = Some(Vec::new());
                    super::super::dispense_fixture::start_recording();
                    game.tick_armor_stands().unwrap();
                    let events = super::super::dispense_fixture::stop_recording();
                    assert_eq!(
                        game.armor_stands[0].equipment[5],
                        Some(ItemStack::new("minecraft:iron_helmet", 1)),
                        "real listener/handle write control",
                    );
                    assert!(!game.armor_stands[0].first_tick);
                    let sounds = game.audio.capture.as_ref().unwrap().len();
                    let equips = events
                        .iter()
                        .filter(|e| e.ends_with("minecraft:equip"))
                        .count();
                    assert_eq!((sounds, equips), (1, 1));
                });
            },
        );
    }

    #[test]
    fn air_and_zero_count_are_empty_on_write() {
        assert!(equipment_stack(Some(ItemStack::new("minecraft:air", 1))).is_none());
        assert!(equipment_stack(Some(ItemStack::new("minecraft:stone", 0))).is_none());
        assert!(equipment_stack(Some(ItemStack::new("minecraft:stone", 1))).is_some());
    }

    #[test]
    fn a_players_slots_come_from_its_inventory_armour_and_off_hand() {
        let stack = |id: &str| Some(ItemStack::new(id, 1));
        let inventory = vec![stack("minecraft:stone"), stack("minecraft:dirt")];
        let armor = [
            stack("minecraft:iron_helmet"),
            None,
            None,
            stack("minecraft:iron_boots"),
        ];
        let offhand = stack("minecraft:shield");
        let mut equipment = horizon_items::equipment::Equipment::default();
        equipment.set(horizon_items::equipment::SADDLE, stack("minecraft:saddle"));
        let at =
            |slot| player_slot(&inventory, 1, &armor, &offhand, &equipment, slot).map(|s| s.item);
        assert_eq!(
            at(EquipmentSlot::MainHand).as_deref(),
            Some("minecraft:dirt")
        );
        assert_eq!(
            at(EquipmentSlot::OffHand).as_deref(),
            Some("minecraft:shield")
        );
        assert_eq!(
            at(EquipmentSlot::Head).as_deref(),
            Some("minecraft:iron_helmet")
        );
        assert_eq!(
            at(EquipmentSlot::Feet).as_deref(),
            Some("minecraft:iron_boots")
        );
        assert_eq!(at(EquipmentSlot::Chest), None);
        assert_eq!(at(EquipmentSlot::Body), None);
        assert_eq!(
            at(EquipmentSlot::Saddle).as_deref(),
            Some("minecraft:saddle")
        );
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_wolf_throws_and_reads_tamed_and_owner() {
        use super::super::{
            Audio, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
        };
        use horizon_client::{chunk::Chunk, entities::ArmorStand};
        use horizon_core::geometry::Vec3;
        use mod_api::Authority;

        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        let chunk = std::sync::Arc::new(Chunk::empty(0, 0));
        game.world.chunks.insert((0, 0), chunk);
        game.native_lifecycle
            .begin_open(Authority::IntegratedServer);
        let id = game
            .spawn_mob(
                "minecraft:wolf",
                [2.5, 64.0, 2.5],
                true,
                super::super::creatures::SpawnSite::Finalizing(
                    horizon_client::mobs::spawn_rules::Reason::Command,
                ),
            )
            .unwrap();
        let wolf = game.living(game.entity(id).unwrap()).unwrap();
        assert_eq!(game.living_is_tamed(wolf), Some(false));
        assert_eq!(game.living_owner(wolf), None);
        let m = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
        (m.extra.tame, m.extra.owner) = (true, Some([1, 2, 3, 4]));
        assert_eq!(game.living_is_tamed(wolf), Some(true));
        assert_eq!(
            game.living_owner(wolf),
            Some(0x1_0000_0002_0000_0003_0000_0004)
        );

        let bone = ItemStack::new("minecraft:bone", 1);
        assert!(
            game.living_drop(wolf, ItemStack::new("minecraft:air", 1), false, true)
                .is_none()
        );
        let item = game.living_drop(wolf, bone.clone(), false, true).unwrap();
        assert_eq!(
            game.entity_kind(item).map(|kind| kind.id(&game)).as_deref(),
            Some("minecraft:item")
        );
        let wolf_uuid = game.uuid_for(id);
        let thrown = game.dropped.last().unwrap();
        let eye = game.mobs.iter().find(|m| m.id == id).unwrap().eye_y();
        assert_eq!(thrown.position.y, eye - f64::from(0.3f32));
        assert_eq!((thrown.pickup_delay, thrown.thrower), (40, Some(wolf_uuid)));
        // The drop swung: one tick on, that swing goes on.
        let m = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
        assert!(m.ai.swinging);
        m.ai.swing_time = 1;
        assert!(!game.living_swing(wolf, Hand::Main, true));
        let scattered = game.living_drop(wolf, bone, true, false).unwrap();
        assert_ne!(scattered, item);
        assert_eq!(game.dropped.last().unwrap().velocity.y, f64::from(0.2f32));
        assert_eq!(game.dropped.last().unwrap().thrower, None);

        game.armor_stands
            .push(ArmorStand::new(90, Vec3::new(0.5, 64.0, 0.5), 0.0));
        let stand = game.living(game.entity(90).unwrap()).unwrap();
        assert_eq!(game.living_is_tamed(stand), None);
        let apple = ItemStack::new("minecraft:apple", 1);
        assert!(game.living_drop(stand, apple, false, false).is_none());
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn player_mod_drop_matches_drop_key_motion() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(
                horizon_client::motion::PlayerMotion::new(
                    game.motion,
                    horizon_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            game.yaw = 37.0;
            game.pitch = -21.0;
            *game.world.random = horizon_core::random::LegacyRandom::new(12345);
            let item = ItemStack::new("minecraft:apple", 1);
            game.inventory[game.selected] = Some(item.clone());
            game.drop_held(true);
            let key_velocity = game.dropped.last().unwrap().velocity;
            *game.world.random = horizon_core::random::LegacyRandom::new(12345);
            crate::game::native_entity_refs::tests::bind_local_player(game, 42, 123);
            let player = game.living(game.entity(42).unwrap()).unwrap();
            let id = game.living_drop(player, item, false, true).unwrap().id();
            let mod_velocity = game
                .dropped
                .iter()
                .find(|entity| entity.id == id)
                .unwrap()
                .velocity;
            assert_eq!(key_velocity, mod_velocity);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn armor_stand_drop_does_not_swing() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.armor_stands
                .push(horizon_client::entities::ArmorStand::new(
                    90,
                    horizon_core::geometry::Vec3::ZERO,
                    0.0,
                ));
            let stand = game.living(game.entity(90).unwrap()).unwrap();
            let before = game.swings_started;
            assert!(
                game.living_drop(stand, ItemStack::new("minecraft:apple", 1), false, false)
                    .is_none()
            );
            assert_eq!(game.swings_started, before);
            assert!(game.dropped.is_empty());
        });
    }

    #[test]
    fn every_equipment_slot_has_its_own_item_command_name() {
        let names = EquipmentSlot::ALL.map(slot_name);
        for (i, n) in names.iter().enumerate() {
            assert!(!names[..i].contains(n), "{n} named twice");
        }
    }
    /// The living reads of #943 answer the engine's own decisions.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_engines_own_portal_melee_freeze_and_air_answers() {
        use horizon_client::mobs::Mob;
        use horizon_client::motion::PlayerMotion;
        use horizon_core::geometry::Vec3;
        use mod_api::Host;
        super::super::native_entity_refs::tests::with_game(|game| {
            // Two zombies an arm's length apart, and one a stride away.
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(30, zombie.clone(), Vec3::new(0.5, 64.0, 0.5), 0.0));
            game.mobs
                .push(Mob::new(31, zombie.clone(), Vec3::new(1.5, 64.0, 0.5), 0.0));
            game.mobs
                .push(Mob::new(32, zombie, Vec3::new(5.5, 64.0, 0.5), 0.0));
            let near = game.living(game.entity(30).unwrap()).unwrap();
            let beside = game.entity(31).unwrap();
            let far = game.entity(32).unwrap();

            // The walk gate's own answers: alive and travelling, until dead.
            {
                let host: &dyn Host = game;
                assert_eq!(near.can_use_portal(host), Some(true));
            }
            game.mobs[0].health = 0.0;
            {
                let host: &dyn Host = game;
                assert_eq!(near.can_use_portal(host), Some(false));
            }
            game.mobs[0].health = 20.0;

            // `LivingEntity.canUsePortal`'s sleep half: a villager asleep in
            // its bed does not travel, awake again it does.
            let villager = game
                .res
                .mob_types()
                .get("minecraft:villager")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(35, villager, Vec3::new(11.5, 64.0, 0.5), 0.0));
            let sleeper = game.living(game.entity(35).unwrap()).unwrap();
            game.mobs.last_mut().unwrap().extra.ints[1] = 1;
            {
                let host: &dyn Host = game;
                assert_eq!(sleeper.can_use_portal(host), Some(false));
            }
            game.mobs.last_mut().unwrap().extra.ints[1] = 0;
            {
                let host: &dyn Host = game;
                assert_eq!(sleeper.can_use_portal(host), Some(true));
            }

            // The melee goal's own reach: beside it, yes; a stride away, no.
            // The freeze tick's decision, and a freeze-immune wearable on it.
            // The air tick's gate: a zombie drowns.
            {
                let host: &dyn Host = game;
                assert_eq!(near.in_melee_range(host, beside), Some(true));
                assert_eq!(near.in_melee_range(host, far), Some(false));
                assert_eq!(near.can_freeze(host), Some(true));
                // The air tick's gate: whatever the species table holds for it.
                assert_eq!(
                    near.can_breathe_underwater(host),
                    Some(game.mobs[0].kind.flags.breathes_water)
                );
            }
            game.mobs[0].extra.equipment.set(
                horizon_items::equipment::FEET,
                Some(horizon_client::items::ItemStack::new(
                    "minecraft:leather_boots",
                    1,
                )),
            );
            {
                let host: &dyn Host = game;
                assert_eq!(near.can_freeze(host), Some(false));
            }

            // A player's own answers: alive and awake, freezable, drowning.
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.living(game.entity(20).unwrap()).unwrap();
            {
                let host: &dyn Host = game;
                assert_eq!(player.can_use_portal(host), Some(true));
                assert_eq!(player.can_freeze(host), Some(true));
                assert_eq!(player.can_breathe_underwater(host), Some(false));
            }
            game.player.as_mut().unwrap().sleeping = true;
            {
                let host: &dyn Host = game;
                assert_eq!(player.can_use_portal(host), Some(false));
            }
            game.player.as_mut().unwrap().sleeping = false;
            game.player.as_mut().unwrap().freeze_immune = true;
            {
                let host: &dyn Host = game;
                assert_eq!(player.can_freeze(host), Some(false));
            }

            let squid = game.res.mob_types().get("minecraft:squid").unwrap().clone();
            game.mobs
                .push(Mob::new(33, squid, Vec3::new(7.5, 64.0, 0.5), 0.0));
            let squid = game.living(game.entity(33).unwrap()).unwrap();
            let golem_kind = game
                .res
                .mob_types()
                .get("minecraft:copper_golem")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(34, golem_kind, Vec3::new(9.5, 64.0, 0.5), 0.0));
            let golem = game.living(game.entity(34).unwrap()).unwrap();
            {
                let host: &dyn Host = game;
                // A squid and a copper golem (#can_breathe_under_water)
                // breathe water.
                assert_eq!(squid.can_breathe_underwater(host), Some(true));
                assert_eq!(golem.can_breathe_underwater(host), Some(true));
            }
            game.mobs.clear();
            {
                let host: &dyn Host = game;
                assert_eq!(near.can_use_portal(host), None);
                assert_eq!(near.in_melee_range(host, beside), None);
                assert_eq!(near.can_freeze(host), None);
                assert_eq!(near.can_breathe_underwater(host), None);
            }
        });
    }

    /// A sleeper's bed orientation is the bed block's own facing (#943):
    /// the block at the sleeping position, where it is one of `#minecraft:beds`.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_sleepers_bed_orientation() {
        use horizon_client::mobs::Mob;
        use horizon_core::geometry::Vec3;
        use mod_api::{Direction, Host};
        super::super::native_entity_refs::tests::with_game(|game| {
            // A floor with a red bed on it, facing north.
            for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
                let mut chunk = horizon_client::chunk::Chunk::empty(x, z);
                chunk.lit = true;
                game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
            }
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            for x in -8..8 {
                for z in -8..8 {
                    game.world.set(x, 63, z, stone).unwrap();
                }
            }
            let bed = game
                .pack
                .assets
                .registry
                .default_state("minecraft:red_bed")
                .unwrap();
            game.world.set(2, 63, 2, bed).unwrap();
            game.world.set(3, 63, 2, bed).unwrap();

            let villager = game
                .res
                .mob_types()
                .get("minecraft:villager")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(30, villager, Vec3::new(0.5, 64.0, 0.5), 0.0));
            let sleeper = game.living(game.entity(30).unwrap()).unwrap();
            let host: &dyn Host = game;
            // Awake: no sleeping position, no bed.
            assert_eq!(sleeper.bed_orientation(host), None);
            // Asleep in the bed: the block's own facing, the engine's own
            // sleeping position naming the bed.
            game.mobs[0].extra.ints[1] = 1;
            horizon_client::mobs::villager::ext_mut(&mut game.mobs[0]).sleeping_pos =
                Some([2, 63, 2]);
            let host: &dyn Host = game;
            assert_eq!(sleeper.bed_orientation(host), Some(Direction::North));
            assert_eq!(sleeper.can_use_portal(host), Some(false));
            // A sleeping position that is not a bed answers none.
            horizon_client::mobs::villager::ext_mut(&mut game.mobs[0]).sleeping_pos =
                Some([0, 63, 0]);
            let host: &dyn Host = game;
            assert_eq!(sleeper.bed_orientation(host), None);
            // A player asleep in the same bed answers the same facing.
            game.player =
                Some(horizon_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            game.sleeping = Some([2, 63, 2]);
            let player = game.living(game.entity(20).unwrap()).unwrap();
            let host: &dyn Host = game;
            assert_eq!(player.bed_orientation(host), Some(Direction::North));
            // Fox sleeping overrides LivingEntity.isSleeping. These species
            // may all travel awake; the fox sleep flag vetoes portal use.
            for name in ["minecraft:fox", "minecraft:bat", "minecraft:cat"] {
                let kind = game.res.mob_types().get(name).unwrap().clone();
                let mut mob = Mob::new(31, kind, Vec3::ZERO, 0.0);
                assert!(horizon_client::mobs::mob_can_use_portal(&mob));
                if name == "minecraft:fox" {
                    mob.extra.ints[0] |= horizon_client::mobs::fox::SLEEPING;
                    assert!(!horizon_client::mobs::mob_can_use_portal(&mob));
                }
            }
            assert!(!game.mob_portal_eligible(&game.mobs[0]));
            game.mobs.clear();
            let host: &dyn Host = game;
            assert_eq!(sleeper.bed_orientation(host), None);
        });
    }

    /// #943, review: a guest answers the same one portal rule over their
    /// body and vehicle (`can_use_portal_of`), not a hand-rolled copy.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_guest_answers_the_one_portal_rule() {
        use mod_api::Host;
        super::super::native_guest_turn_tests::with_guests(|game| {
            // The fixture leaves the guest without a body: give one, as a
            // joined player's server record keeps.
            game.lan.as_mut().unwrap().hub.update_guest(12, |g| {
                g.motion = Some(Box::new(
                    horizon_client::motion::PlayerMotion::new(
                        game.motion,
                        horizon_core::geometry::Vec3::new(3.0, 64.0, 5.0),
                    )
                    .unwrap(),
                ))
            });
            let living = game.living(game.entity(12).unwrap()).unwrap();
            let host: &dyn Host = game;
            assert_eq!(living.can_use_portal(host), Some(true));
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| g.vehicle = Some(99));
            let host: &dyn Host = game;
            assert_eq!(living.can_use_portal(host), Some(false));
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| g.vehicle = None);
            // Asleep: the rule's sleep half answers, over their body.
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| g.sleeping = true);
            let host: &dyn Host = game;
            assert_eq!(living.can_use_portal(host), Some(false));
            // A passenger crosses with its vehicle, not as its own
            // traveller: the rule's riding half answers false.
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| g.sleeping = false);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| g.vehicle = Some(60));
            let host: &dyn Host = game;
            assert_eq!(living.can_use_portal(host), Some(false));
        });
    }
}
