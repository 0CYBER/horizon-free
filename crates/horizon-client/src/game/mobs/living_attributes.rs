//! The item modifiers of the live attribute maps of mobs, armor stands and the
//! dragon (`LivingEntity.collectEquipmentChanges`). The map and the NBT are in
//! `horizon_client::living_attributes`.
use super::Game;
use super::commands::Target;
use horizon_client::mobs::{
    Mob,
    finalize::{self, Attr, Op},
};
use horizon_items::items::Items;
use horizon_world::attributes::{
    AttributeError, Instance, Modifier, Observer, Operation,
    entity::{AttributeId, Catalog},
};
use std::convert::Infallible;
use std::sync::Arc;

/// One mob's `collectEquipmentChanges`: each slot whose stack is not the one
/// the map holds swaps the old stack's attribute modifiers for the new one's,
/// full stacks with their components compared, as the tick's pass and a write
/// both run it. The map is taken out so the seed can read the mob.
pub(in crate::game) fn collect_equipment(catalog: &Arc<Catalog>, items: &Items, m: &mut Mob) {
    horizon_client::mobs::armor::prepare(m, catalog);
    let mut live = std::mem::take(&mut m.extra.live);
    let seed = || {
        horizon_client::entity_nbt::attribute_entries(m)
            .into_iter()
            .map(|(_, entry)| entry)
            .collect()
    };
    let changed =
        live.collect_equipment_changes(catalog, &m.kind.id, &m.extra.equipment, items, seed);
    warn_failed(m.id, &m.kind.id, changed);
    m.extra.live = live;
}

/// The attribute `name` when it is one a mob keeps itself: `finalize`'s store,
/// which its gameplay reads and its save writes, for the attributes its type has.
fn own_attribute(m: &Mob, name: &str) -> Option<Attr> {
    let short = name.strip_prefix("minecraft:").unwrap_or(name);
    finalize::command_attr(short).filter(|a| finalize::default_value(m, *a).is_some())
}

/// `write` on one of the mob's own attributes. A lower max health marks the attribute dirty;
/// the mob's next tick cuts its health (`LivingEntity.refreshDirtyAttributes`, `finalize::refresh`).
fn write_own_attribute(m: &mut Mob, a: Attr, write: AttributeWrite) -> bool {
    write.apply_to_mob(m, a)
}

/// An attribute write on a mob that is in no level yet (`finalize_spawn`'s
/// lent mob), then its equipment's modifiers collected again, as
/// `Game::write_attribute` does for a joined one. Only the attributes the mob
/// keeps itself take it: the rest live in maps the level keeps per joined
/// entity. None for another attribute, `Some(false)` when an add finds the id
/// taken or a remove finds none.
pub(in crate::game) fn write_unspawned_attribute(
    catalog: &Arc<Catalog>,
    items: &Items,
    m: &mut Mob,
    name: &str,
    write: AttributeWrite,
) -> Option<bool> {
    let done = write_own_attribute(m, own_attribute(m, name)?, write);
    collect_equipment(catalog, items, m);
    Some(done)
}

impl Game<'_> {
    /// `LivingEntity.tick`'s `collectEquipmentChanges` for what the server
    /// simulates besides players: each slot whose item is not the one the map
    /// holds swaps the old item's attribute modifiers for the new one's. An
    /// entity that changed nothing costs a compare of its eight slots, and
    /// one that wears nothing and never wore anything is skipped outright.
    pub(super) fn tick_equipment_attributes(&mut self) {
        if self.remote.is_some() || self.options.execution_role.is_client() {
            return;
        }
        let catalog = self.motion.attributes.clone();
        let items = world_items!(self);
        for m in self.mobs.iter_mut().filter(|m| !m.removed) {
            if m.extra.equipment.is_empty() && !m.extra.live.has_worn() {
                continue;
            }
            collect_equipment(&catalog, items, m);
        }
        for i in 0..self.armor_stands.len() {
            // Looked at without a `&mut`, which the list counts as a touch: most stands wear
            // nothing and never did.
            let stand = &self.armor_stands[i];
            if stand.equipment.is_empty() && !stand.live.has_worn() {
                continue;
            }
            let a = &mut self.armor_stands[i];
            let changed = a.live.collect_equipment_changes(
                &catalog,
                "minecraft:armor_stand",
                &a.equipment,
                items,
                Vec::new,
            );
            warn_failed(a.id, "minecraft:armor_stand", changed);
        }
        if let Some(d) = self.dragon.as_mut()
            && !(d.equipment.is_empty() && !d.live.has_worn())
        {
            let changed = d.live.collect_equipment_changes(
                &catalog,
                "minecraft:ender_dragon",
                &d.equipment,
                items,
                Vec::new,
            );
            warn_failed(d.id, "minecraft:ender_dragon", changed);
        }
    }

    /// `collectEquipmentChanges` of the mob `id` right away: equipment a mod
    /// wrote through a handle takes its attribute modifiers on the live map
    /// at once — its armor and the reads of arismoko/horizon#940 — not at the tick's pass.
    /// The tick's own pass remains the safety net for the other writers.
    pub(super) fn collect_mob_equipment_attributes(&mut self, id: u32) {
        if self.remote.is_some() || self.options.execution_role.is_client() {
            return;
        }
        let catalog = self.motion.attributes.clone();
        let items = world_items!(self);
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed) else {
            return;
        };
        collect_equipment(&catalog, items, m);
    }
}

/// An item modifier the map refused (a bad modifier id) is logged, naming the
/// entity, instead of being lost.
fn warn_failed(id: u32, kind: &str, result: horizon_client::Result<()>) {
    if let Err(error) = result {
        log::warn!("equipment attributes of {kind} {id}: {error}");
    }
}

/// One attribute write: the one path of `/attribute` (`AttributeCommand`'s
/// `base set`, `base reset`, `modifier add` and `modifier remove`) and the mod
/// API's `LivingRef` writes, through `Game::write_attribute`.
#[derive(Clone)]
pub(in crate::game) enum AttributeWrite {
    /// `addTransientModifier` or `addPermanentModifier`, refused when the id
    /// is taken; with `replace`, the modifier with that id goes first
    /// (`addOrUpdateTransientModifier`, `addOrReplacePermanentModifier`).
    Add {
        modifier: Modifier,
        permanent: bool,
        replace: bool,
    },
    /// `removeModifier`.
    Remove(String),
    /// `setBaseValue`.
    Base(f64),
    /// `setBaseValue` of the entity type's default.
    ResetBase,
}

impl AttributeWrite {
    /// An add of the modifier `id`, or None for an id `Modifier` refuses.
    pub(in crate::game) fn add(
        id: &str,
        amount: f64,
        operation: Operation,
        permanent: bool,
        replace: bool,
    ) -> Option<Self> {
        Some(Self::Add {
            modifier: Modifier::new(id, amount, operation).ok()?,
            permanent,
            replace,
        })
    }

    /// The write on an instance, whose type's default base is `default`:
    /// false when an add finds the id taken or a remove finds nothing.
    fn apply<O: Observer<Error = Infallible>>(
        self,
        value: &mut Instance,
        observer: &mut O,
        default: f64,
    ) -> Result<bool, AttributeError<Infallible>> {
        match self {
            Self::Add {
                modifier,
                permanent,
                replace,
            } => {
                if value.modifier(modifier.id()).is_some() {
                    if !replace {
                        return Ok(false);
                    }
                    value.remove(modifier.id(), observer)?;
                }
                if permanent {
                    value.add_permanent(modifier, observer)?;
                } else {
                    value.add_transient(modifier, observer)?;
                }
                Ok(true)
            }
            Self::Remove(id) => value.remove(&id, observer),
            Self::Base(base) => {
                value.set_base_value(base, observer)?;
                Ok(true)
            }
            Self::ResetBase => {
                value.set_base_value(default, observer)?;
                Ok(true)
            }
        }
    }

    /// The write on one of the attributes a mob's gameplay reads from
    /// `finalize`, which saves its bases and permanent modifiers.
    fn apply_to_mob(self, m: &mut Mob, a: Attr) -> bool {
        let taken = |m: &Mob, id: &str| {
            finalize::has_modifier(m, a, id)
                || finalize::has_transient_modifier(m, a, id)
                || (id == "minecraft:baby" && finalize::transient(m, a).is_some())
        };
        let remove = |m: &mut Mob, id: &str| {
            if id == "minecraft:baby" && finalize::transient(m, a).is_some() {
                m.extra
                    .attributes
                    .get_or_insert_with(Box::default)
                    .baby_speed_removed = true;
            }
            finalize::remove_modifier(m, a, id);
        };
        match self {
            Self::Add {
                modifier,
                permanent,
                replace,
            } => {
                let id = modifier.id();
                if taken(m, id) {
                    if !replace {
                        return false;
                    }
                    remove(m, id);
                }
                let op = match modifier.operation() {
                    Operation::AddValue => Op::AddValue,
                    Operation::AddMultipliedBase => Op::AddMultipliedBase,
                    Operation::AddMultipliedTotal => Op::AddMultipliedTotal,
                };
                if permanent {
                    finalize::add_modifier(m, a, id.to_owned(), modifier.amount(), op);
                } else {
                    finalize::add_transient_modifier(m, a, id, modifier.amount(), op);
                }
                true
            }
            Self::Remove(id) => {
                let had = taken(m, &id);
                if had {
                    remove(m, &id);
                }
                had
            }
            Self::Base(base) => {
                finalize::set_base(m, a, base);
                true
            }
            Self::ResetBase => {
                let default = finalize::default_value(m, a).unwrap_or_default();
                finalize::set_base(m, a, default);
                true
            }
        }
    }

    /// What a live map keeps of the transient modifiers written to it after
    /// this write: the added one, or None to forget the id.
    fn transient_record(&self) -> Option<(&str, Option<Modifier>)> {
        match self {
            Self::Add {
                modifier,
                permanent,
                ..
            } => Some((modifier.id(), (!permanent).then(|| modifier.clone()))),
            Self::Remove(id) => Some((id, None)),
            Self::Base(_) | Self::ResetBase => None,
        }
    }
}

impl Game<'_> {
    /// Applies `write` to the attribute `name` of `t` on the record that the
    /// entity's gameplay and its clients' attribute sync read:
    /// - a player's (or a guest's, in its turn) own attribute map;
    /// - for a mob, `finalize`'s store for the nine attributes its behavior
    ///   reads there (so it moves and hits by them at once); for its armor and
    ///   toughness its armor map (`mobs/armor.rs`); for the rest, as for an
    ///   armor stand, its generic map (`cmds.attributes`), which the sync sends.
    ///
    /// A mob's and a stand's live map (`living_attributes`, which the mod
    /// API's reads and the save use) takes the same write and keeps it
    /// through a rebuild. A lower max health cuts a mob's health
    /// (`LivingEntity.onAttributeUpdated`).
    ///
    /// None when the entity has no such attribute or takes no attribute writes
    /// (the dragon, as `/attribute` refuses it); `Some(false)` when an add
    /// finds the id taken or a remove finds none.
    pub(in crate::game) fn write_attribute(
        &mut self,
        t: Target,
        name: &str,
        write: AttributeWrite,
    ) -> Option<bool> {
        let done = self.apply_attribute_write(t, name, write)?;
        // A finalize write drops the mob's live map (its seed changed), which
        // takes the equipment's modifiers with it: collect them again at
        // once, so the next read — a hit's armor, the attribute sync — sees
        // them (arismoko/horizon#941).
        if let Target::Mob(id) = t {
            self.collect_mob_equipment_attributes(id);
        }
        Some(done)
    }

    fn apply_attribute_write(
        &mut self,
        t: Target,
        name: &str,
        write: AttributeWrite,
    ) -> Option<bool> {
        let catalog = self.motion.attributes.clone();
        let id = catalog.id(name)?;
        let default = match write {
            AttributeWrite::ResetBase => catalog.default_base(&self.target_type(t), id)?,
            _ => 0.0,
        };
        match t {
            Target::Player | Target::Guest(_) => self
                .for_player(t, |g| {
                    let live = &mut g.player.as_mut()?.attributes;
                    let handle = live.get_instance(Some(live.key(name)?)).ok()??;
                    live.edit(&handle, |value, observer| {
                        write.apply(value, observer, default)
                    })
                    .ok()
                })
                .flatten(),
            Target::Mob(mob) => {
                if self.sulfur_archetype_attribute(t, name) {
                    let done = self.write_generic_attribute(t, name, id, write.clone(), default)?;
                    // The body's map owns the command value, while bodiless physics
                    // reads the base and permanent modifiers from finalize's cache.
                    if done && name == "minecraft:knockback_resistance" {
                        let m = self.mobs.iter_mut().find(|m| m.id == mob && !m.removed)?;
                        // Admission belongs to the body's map. Its archetype changes can
                        // remove an ID without touching finalize's mirror, so an accepted
                        // add replaces any stale copy there rather than being refused twice.
                        let mirror = match write {
                            AttributeWrite::Add {
                                modifier,
                                permanent,
                                ..
                            } => AttributeWrite::Add {
                                modifier,
                                permanent,
                                replace: true,
                            },
                            other => other,
                        };
                        write_own_attribute(m, Attr::KnockbackResistance, mirror);
                    }
                    return Some(done);
                }
                let m = self.mobs.iter_mut().find(|m| m.id == mob && !m.removed)?;
                if let Some(a) = own_attribute(m, name) {
                    return Some(write_own_attribute(m, a, write));
                }
                self.write_generic_attribute(t, name, id, write, default)
            }
            Target::Stand(_) => self.write_generic_attribute(t, name, id, write, default),
            _ => None,
        }
    }

    fn write_generic_attribute(
        &mut self,
        t: Target,
        name: &str,
        id: AttributeId,
        write: AttributeWrite,
        default: f64,
    ) -> Option<bool> {
        let map = self.generic_attributes(t, name)?;
        if !map.has(id) {
            return None;
        }
        let done = map
            .edit(id, |value, observer| {
                write.clone().apply(value, observer, default)
            })?
            .ok()?;
        if done {
            self.write_live_attributes(t, name, write, default);
        }
        Some(done)
    }

    /// The same write on a mob's or a stand's live map: its base and
    /// permanent modifiers go to the map's saved entries, a transient modifier
    /// to its own list, so a rebuilt map has them again.
    fn write_live_attributes(
        &mut self,
        t: Target,
        name: &str,
        write: AttributeWrite,
        default: f64,
    ) {
        let catalog = self.motion.attributes.clone();
        let (live, entity, seed) = match t {
            Target::Mob(id) => {
                let Some(m) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed) else {
                    return;
                };
                // The seed is only read when the map is not built yet.
                let seed = if m.extra.live.built().is_none() {
                    horizon_client::entity_nbt::attribute_entries(m)
                        .into_iter()
                        .map(|(_, entry)| entry)
                        .collect()
                } else {
                    Vec::new()
                };
                (&mut m.extra.live, m.kind.id.clone(), seed)
            }
            Target::Stand(id) => {
                let Some(a) = self.armor_stands.iter_mut().find(|a| a.id == id) else {
                    return;
                };
                (&mut a.live, "minecraft:armor_stand".to_owned(), Vec::new())
            }
            _ => return,
        };
        let record = write.transient_record().map(|(id, m)| (id.to_owned(), m));
        let done = live
            .edit(
                &catalog,
                &entity,
                name,
                true,
                || seed,
                |value, observer| write.apply(value, observer, default),
            )
            .unwrap_or(false);
        if done && let Some((id, modifier)) = record {
            live.keep_transient(name, &id, modifier);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_game;
    use horizon_client::{entities::ArmorStand, items::ItemStack, mobs::Mob};
    use horizon_core::geometry::Vec3;
    use horizon_items::equipment::{CHEST, HEAD, MAINHAND};

    fn stack(item: &str) -> Option<ItemStack> {
        Some(ItemStack::new(item, 1))
    }

    /// A zombie, an armor stand and the dragon, ids 12, 13 and 14.
    fn spawn(game: &mut Game<'_>) {
        let zombie = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
        game.armor_stands.push(ArmorStand::new(13, Vec3::ZERO, 0.0));
        let mut dragon = crate::game::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
        dragon.id = 14;
        game.dragon.replace(dragon);
    }

    fn read(game: &Game<'_>, id: u32, name: &str) -> (Option<f64>, Option<f64>) {
        let living = game.living(game.entity(id).unwrap()).unwrap();
        (
            living.attribute_value(game, name),
            living.attribute_base(game, name),
        )
    }

    /// A held stack with its own `attribute_modifiers` hits by them, not by
    /// its item type's defaults (arismoko/horizon#941).
    #[test]
    #[ignore = "daily"]
    fn a_changed_held_stack_hits_by_its_own_modifiers() {
        use horizon_client::mobs::gear;
        use horizon_core::nbt::Tag;
        use mod_api::EquipmentSlot;
        with_game(|game| {
            spawn(game);
            let zombie = living(game, 12);
            let attack = |game: &Game<'_>| {
                gear::attack_damage(
                    game.mobs.iter().find(|m| m.id == 12).unwrap(),
                    Some(game.res.items()),
                )
            };
            assert_eq!(attack(game), 3.0, "a zombie's own attack damage");
            let mut own = ItemStack::new("minecraft:diamond_sword", 1);
            own.data_mut().set_other(
                "minecraft:attribute_modifiers",
                Some(Tag::List(vec![Tag::compound([
                    ("type", Tag::String("minecraft:attack_damage".into())),
                    ("id", Tag::String("test:dull".into())),
                    ("amount", Tag::Double(1.0f64.to_bits())),
                    ("operation", Tag::String("add_value".into())),
                    ("slot", Tag::String("mainhand".into())),
                ])])),
            );
            assert!(
                game.living_set_equipment(zombie, EquipmentSlot::MainHand, Some(own)),
                "the write takes"
            );
            assert_eq!(attack(game), 4.0, "the stack's own +1, not the sword's +6");
            assert!(game.living_set_equipment(
                zombie,
                EquipmentSlot::MainHand,
                stack("minecraft:diamond_sword")
            ));
            assert_eq!(attack(game), 9.0, "the plain sword's own +6");
            assert!(game.living_set_equipment(zombie, EquipmentSlot::MainHand, None));
            assert_eq!(attack(game), 3.0);
        });
    }

    /// The mob's attribute packet is its live map's instances (arismoko/horizon#941): base
    /// and each modifier once — the equipment's own rows and `/attribute`'s
    /// writes among them — with no modifier applied twice, and no instance
    /// for an attribute nothing created.
    #[test]
    #[ignore = "daily"]
    fn the_mob_attribute_packet_is_the_live_maps_instances() {
        use horizon_core::nbt::Tag;
        use mod_api::EquipmentSlot;
        with_game(|game| {
            spawn(game);
            let zombie = living(game, 12);
            let mut own = ItemStack::new("minecraft:diamond_chestplate", 1);
            own.data_mut().set_other(
                "minecraft:attribute_modifiers",
                Some(Tag::List(vec![Tag::compound([
                    ("type", Tag::String("minecraft:armor".into())),
                    ("id", Tag::String("test:thin".into())),
                    ("amount", Tag::Double(4.0f64.to_bits())),
                    ("operation", Tag::String("add_value".into())),
                    ("slot", Tag::String("chest".into())),
                ])])),
            );
            assert!(game.living_set_equipment(zombie, EquipmentSlot::Chest, Some(own)));
            // A finalize write, whose rebuild keeps the equipment's rows.
            assert!(zombie.set_attribute_base(game, "minecraft:max_health", 25.0));
            let mut synced = Vec::new();
            let mob = game.mobs[0].clone();
            game.mob_attributes(
                &mob,
                &mut crate::game::lan_attributes::SpeedByType::default(),
                &mut synced,
            );
            // Each attribute once.
            let mut ids: Vec<_> = synced.iter().map(|s| s.attribute).collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), synced.len(), "{synced:?}");
            let of = |name: &str| {
                let id = game.motion.attributes.id(name).unwrap().registry_id();
                synced.iter().find(move |s| s.attribute == id)
            };
            // Armor: the zombie's base and only the stack's own row.
            let armor = of("minecraft:armor").unwrap();
            assert_eq!(armor.base, 2.0);
            assert_eq!(
                armor.modifiers,
                [horizon_protocol::packets::play::AttributeModifier {
                    id: "test:thin".into(),
                    amount: 4.0,
                    operation: 0,
                }]
            );
            // The written max health base, as the map holds it.
            let health = of("minecraft:max_health").unwrap();
            assert_eq!(health.base, 25.0);
            assert!(health.modifiers.is_empty());
            // Movement speed: the instance every mob's first step creates.
            let speed = of("minecraft:movement_speed").unwrap();
            assert!(speed.base > 0.0, "{speed:?}");
            assert!(speed.modifiers.is_empty());
            // Nothing else: an attribute nothing created sends no instance.
            assert_eq!(synced.len(), 3, "{synced:?}");
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_base_write_preserves_covered_for_damage_and_attribute_sync() {
        with_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:shulker")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, kind, Vec3::ZERO, 0.0));
            let catalog = game.motion.attributes.clone();
            let m = &mut game.mobs[0];
            // The permanent row installed by Shulker.setRawPeekAmount(0).
            m.extra
                .live
                .edit(
                    &catalog,
                    &m.kind.id,
                    "minecraft:armor",
                    true,
                    Vec::new,
                    |value, observer| {
                        value.add_permanent(
                            Modifier::new("minecraft:covered", 20.0, Operation::AddValue).unwrap(),
                            observer,
                        )
                    },
                )
                .unwrap();
            let saved =
                horizon_client::entity_nbt::mob_to_nbt(&game.mobs[0], [0; 4], None, None, |_| None);
            let loaded_id = game.load_mob(saved.clone(), None).unwrap();
            let loaded = game.mobs.iter().find(|m| m.id == loaded_id).unwrap();
            assert_eq!(horizon_client::mobs::armor::values(loaded), (20.0, 0.0));
            game.apply_entity_nbt(Target::Mob(12), saved).unwrap();
            assert_eq!(
                horizon_client::mobs::armor::values(&game.mobs[0]),
                (20.0, 0.0)
            );
            let shulker = living(game, 12);
            assert!(shulker.set_attribute_base(game, "minecraft:max_health", 40.0));
            // No handle read in between: damage must not need a read to restore the map.
            assert_eq!(
                horizon_client::mobs::armor::values(&game.mobs[0]),
                (20.0, 0.0)
            );
            let mut synced = Vec::new();
            let m = game.mobs[0].clone();
            game.mob_attributes(
                &m,
                &mut crate::game::lan_attributes::SpeedByType::default(),
                &mut synced,
            );
            let armor_id = catalog.id("minecraft:armor").unwrap().registry_id();
            let armor = synced.iter().find(|row| row.attribute == armor_id).unwrap();
            assert_eq!(armor.base, 0.0);
            assert_eq!(
                armor.modifiers,
                [horizon_protocol::packets::play::AttributeModifier {
                    id: "minecraft:covered".into(),
                    amount: 20.0,
                    operation: 0,
                }]
            );
        });
    }

    /// The armor a hit reads follows the live map at once (arismoko/horizon#941): a stack a
    /// mod or `/item` changed protects by its own `attribute_modifiers`, a
    /// `/attribute` write takes effect without a tick, a horse's body piece
    /// armors it by its stack's rows, and the save keeps a written base and
    /// modifier.
    #[test]
    #[ignore = "daily"]
    fn a_hits_armor_follows_the_live_map_at_once() {
        use AttributeOperation::*;
        use ModifierLifetime::*;
        use horizon_client::mobs::{Damage, DamageKind, armor};
        use horizon_core::nbt::Tag;
        use mod_api::EquipmentSlot;
        with_game(|game| {
            spawn(game);
            let values =
                |game: &Game<'_>| armor::values(game.mobs.iter().find(|m| m.id == 12).unwrap());
            let zombie = living(game, 12);
            // The zombie's own armor is 2, the diamond chestplate's 8 and 2
            // toughness: the write through the handle collects at once.
            assert!(game.living_set_equipment(
                zombie,
                EquipmentSlot::Chest,
                stack("minecraft:diamond_chestplate")
            ));
            assert_eq!(values(game), (10.0, 2.0));
            // A chestplate with its own `attribute_modifiers` protects by
            // them, not by the diamond one's.
            let mut own = ItemStack::new("minecraft:diamond_chestplate", 1);
            own.data_mut().set_other(
                "minecraft:attribute_modifiers",
                Some(Tag::List(vec![Tag::compound([
                    ("type", Tag::String("minecraft:armor".into())),
                    ("id", Tag::String("test:thin".into())),
                    ("amount", Tag::Double(4.0f64.to_bits())),
                    ("operation", Tag::String("add_value".into())),
                    ("slot", Tag::String("chest".into())),
                ])])),
            );
            assert!(game.living_set_equipment(zombie, EquipmentSlot::Chest, Some(own)));
            assert_eq!(values(game), (6.0, 0.0));
            // A `/attribute` base write lands at once, on top of the stack's.
            assert!(zombie.set_attribute_base(game, "minecraft:armor", 5.0));
            assert_eq!(values(game), (9.0, 0.0));
            // Unequipped, the base and a permanent modifier are all that is
            // left, and the damage boundary reads the same value.
            assert!(zombie.add_attribute_modifier(
                game,
                "minecraft:armor",
                modifier("test:plated", 3.0, AddValue),
                Permanent
            ));
            assert!(game.living_set_equipment(zombie, EquipmentSlot::Chest, None));
            assert_eq!(values(game), (8.0, 0.0));
            let level = crate::game::entity::damage_level_of!(game);
            let m = game.mobs.iter_mut().find(|m| m.id == 12).unwrap();
            m.health = 20.0;
            assert!(m.damage(level, 4.0, Damage::of(DamageKind::Cactus)));
            // CombatRules.getDamageAfterAbsorb: (8 - 4/2) of 25 absorbs.
            let expected = 4.0 * (1.0 - 6.0 / 25.0);
            assert!((f64::from(m.health) - (20.0 - expected)).abs() < 1e-3);
            // The written base and modifier survive the map's own save and
            // load; the first read builds the loaded map again, as a world
            // load's first tick or read does.
            let mut list = Vec::new();
            game.mobs[0].extra.live.save_into(&mut list);
            let mut fresh = horizon_client::living_attributes::LivingAttributes::default();
            fresh.load(Some(&Tag::List(list)), |_| true);
            game.mobs[0].extra.live = fresh;
            read(game, 12, "minecraft:armor");
            assert_eq!(values(game), (8.0, 0.0));
            // A horse's body piece armors it by its stack's rows.
            let horse_kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
            // The fixture dragon owns IDs 14 through 22, including its parts.
            game.mobs.push(Mob::new(40, horse_kind, Vec3::ZERO, 0.0));
            let horse = living(game, 40);
            assert!(game.living_set_equipment(
                horse,
                EquipmentSlot::Body,
                stack("minecraft:diamond_horse_armor")
            ));
            let armored = armor::values(game.mobs.iter().find(|m| m.id == 40).unwrap());
            assert!(
                armored.0 >= 1.0,
                "the body piece armors the horse: {armored:?}"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_zombie_reads_its_defaults() {
        with_game(|game| {
            spawn(game);
            assert_eq!(
                read(game, 12, "minecraft:max_health"),
                (Some(20.0), Some(20.0))
            );
            assert_eq!(
                read(game, 12, "minecraft:attack_damage"),
                (Some(3.0), Some(3.0))
            );
            // Not an attribute of an armor stand.
            assert_eq!(read(game, 13, "minecraft:attack_damage"), (None, None));
        });
    }

    /// A read of a mob that wears nothing builds its map once and keeps it,
    /// so the next reads allocate nothing.
    #[test]
    #[ignore = "daily"]
    fn a_read_builds_the_map_once() {
        with_game(|game| {
            spawn(game);
            assert!(game.mobs[0].extra.live.built().is_none());
            read(game, 12, "minecraft:max_health");
            let first: *const _ = game.mobs[0].extra.live.built().unwrap();
            read(game, 12, "minecraft:attack_damage");
            read(game, 12, "minecraft:max_health");
            let again: *const _ = game.mobs[0].extra.live.built().unwrap();
            assert!(std::ptr::eq(first, again));
            // A changed base drops it; the next read builds it from the new base.
            game.mobs[0].extra.live.invalidate();
            assert!(game.mobs[0].extra.live.built().is_none());
            assert_eq!(
                read(game, 12, "minecraft:max_health"),
                (Some(20.0), Some(20.0))
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_held_sword_raises_the_attack_damage_until_it_is_put_away() {
        with_game(|game| {
            spawn(game);
            let damage = "minecraft:attack_damage";
            game.mobs[0].set_equipment_stack(MAINHAND, stack("minecraft:diamond_sword"));
            game.tick_equipment_attributes();
            // +6 from the sword's `base_attack_damage` modifier; the base stays.
            assert_eq!(read(game, 12, damage), (Some(9.0), Some(3.0)));
            // The same item again changes nothing (no duplicate modifier).
            game.tick_equipment_attributes();
            assert_eq!(read(game, 12, damage), (Some(9.0), Some(3.0)));
            // Another sword takes the first one's modifier off before its own.
            game.mobs[0].set_equipment_stack(MAINHAND, stack("minecraft:netherite_sword"));
            game.tick_equipment_attributes();
            assert_eq!(read(game, 12, damage), (Some(10.0), Some(3.0)));
            game.mobs[0].set_equipment_stack(MAINHAND, None);
            game.tick_equipment_attributes();
            assert_eq!(read(game, 12, damage), (Some(3.0), Some(3.0)));
            // A sword on the head is in no slot its modifiers name.
            game.mobs[0].set_equipment_stack(HEAD, stack("minecraft:diamond_sword"));
            game.tick_equipment_attributes();
            assert_eq!(read(game, 12, damage), (Some(3.0), Some(3.0)));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_stand_and_the_dragon_wear_armor_modifiers() {
        with_game(|game| {
            spawn(game);
            let armor = "minecraft:armor";
            game.armor_stands[0]
                .equipment
                .set(HEAD, stack("minecraft:diamond_helmet"));
            game.dragon
                .as_mut()
                .unwrap()
                .equipment
                .set(CHEST, stack("minecraft:diamond_chestplate"));
            game.tick_equipment_attributes();
            assert_eq!(read(game, 13, armor), (Some(3.0), Some(0.0)));
            assert_eq!(read(game, 14, armor), (Some(8.0), Some(0.0)));
            game.armor_stands[0].equipment.set(HEAD, None);
            game.tick_equipment_attributes();
            assert_eq!(read(game, 13, armor), (Some(0.0), Some(0.0)));
            assert_eq!(read(game, 14, armor), (Some(8.0), Some(0.0)));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_stale_handle_reads_none() {
        with_game(|game| {
            spawn(game);
            let living = game.living(game.entity(12).unwrap()).unwrap();
            assert_eq!(
                living.attribute_value(game, "minecraft:max_health"),
                Some(20.0)
            );
            game.mobs.clear();
            assert_eq!(living.attribute_value(game, "minecraft:max_health"), None);
            assert_eq!(living.attribute_base(game, "minecraft:max_health"), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_saved_base_and_modifier_are_read_and_saved_again() {
        use horizon_core::nbt::Tag;
        with_game(|game| {
            spawn(game);
            let saved = Tag::List(vec![Tag::compound([
                ("id", Tag::String("minecraft:max_health".into())),
                ("base", Tag::Double(40.0f64.to_bits())),
                (
                    "modifiers",
                    Tag::List(vec![Tag::compound([
                        ("id", Tag::String("test:bonus".into())),
                        ("amount", Tag::Double(0.5f64.to_bits())),
                        ("operation", Tag::String("add_multiplied_base".into())),
                    ])]),
                ),
            ])]);
            game.armor_stands[0].live.load(Some(&saved), |_| true);
            assert_eq!(
                read(game, 13, "minecraft:max_health"),
                (Some(60.0), Some(40.0))
            );
            let mut list = Vec::new();
            game.armor_stands[0].live.save_into(&mut list);
            assert_eq!(Tag::List(list), saved);
        });
    }

    /// The live map takes a held stack's own `attribute_modifiers`, and
    /// another stack of the same item with other components is a change.
    #[test]
    #[ignore = "daily"]
    fn a_stacks_own_modifiers_replace_its_items() {
        use horizon_core::nbt::Tag;
        with_game(|game| {
            spawn(game);
            let damage = "minecraft:attack_damage";
            let mut own = ItemStack::new("minecraft:diamond_sword", 1);
            own.data_mut().set_other(
                "minecraft:attribute_modifiers",
                Some(Tag::List(vec![Tag::compound([
                    ("type", Tag::String(damage.into())),
                    ("id", Tag::String("test:dull".into())),
                    ("amount", Tag::Double(1.0f64.to_bits())),
                    ("operation", Tag::String("add_value".into())),
                    ("slot", Tag::String("mainhand".into())),
                ])])),
            );
            game.mobs[0].set_equipment_stack(MAINHAND, Some(own));
            game.tick_equipment_attributes();
            // +1 from its own list, not the sword's +6.
            assert_eq!(read(game, 12, damage), (Some(4.0), Some(3.0)));
            game.mobs[0].set_equipment_stack(MAINHAND, stack("minecraft:diamond_sword"));
            game.tick_equipment_attributes();
            assert_eq!(read(game, 12, damage), (Some(9.0), Some(3.0)));
        });
    }

    /// A mob's gear knockback resistance reads its stacks once, keeps what it
    /// read while the same stacks are worn, and reads them again when one
    /// changes: here a chestplate with its own list.
    #[test]
    #[ignore = "daily"]
    fn gear_knockback_is_read_again_only_when_the_gear_changes() {
        use horizon_core::nbt::Tag;
        with_game(|game| {
            spawn(game);
            let read_at = |game: &Game<'_>| {
                let m = &game.mobs[0];
                let gear = m.extra.gear_knockback.as_deref();
                (gear.map(std::ptr::from_ref), m.gear_knockback_resistance)
            };
            let close = |value: Option<f64>, to: f64| value.is_some_and(|v| (v - to).abs() < 1e-6);
            game.mobs[0].set_equipment_stack(CHEST, stack("minecraft:netherite_chestplate"));
            game.refresh_gear_knockback_resistance();
            let (first, value) = read_at(game);
            assert!(first.is_some() && close(value, 0.1), "{value:?}");
            game.refresh_gear_knockback_resistance();
            assert_eq!(read_at(game).0, first, "the same gear is not read again");
            let mut own = ItemStack::new("minecraft:netherite_chestplate", 1);
            own.data_mut().set_other(
                "minecraft:attribute_modifiers",
                Some(Tag::List(vec![Tag::compound([
                    ("type", Tag::String("minecraft:knockback_resistance".into())),
                    ("id", Tag::String("test:anchor".into())),
                    ("amount", Tag::Double(0.5f64.to_bits())),
                    ("operation", Tag::String("add_value".into())),
                    ("slot", Tag::String("chest".into())),
                ])])),
            );
            game.mobs[0].set_equipment_stack(CHEST, Some(own));
            game.refresh_gear_knockback_resistance();
            let (second, value) = read_at(game);
            assert!(second != first && close(value, 0.5), "{value:?}");
            game.mobs[0].set_equipment_stack(CHEST, None);
            game.refresh_gear_knockback_resistance();
            assert_eq!(read_at(game), (None, None));
        });
    }

    fn modifier(id: &str, amount: f64, operation: AttributeOperation) -> AttributeModifier {
        AttributeModifier::new(Identifier::parse(id).unwrap(), amount, operation)
    }

    fn living(game: &Game<'_>, id: u32) -> mod_api::LivingRef {
        game.living(game.entity(id).unwrap()).unwrap()
    }

    /// The ids of the modifiers a saved `attributes` list holds.
    fn saved_ids<'a>(list: impl IntoIterator<Item = &'a horizon_core::nbt::Tag>) -> Vec<String> {
        use horizon_client_core::nbt_value::text;
        use horizon_core::nbt::Tag;
        list.into_iter()
            .flat_map(|entry| match entry.get("modifiers") {
                Some(Tag::List(l)) => l.iter().filter_map(|m| text(m.get("id"))).collect(),
                _ => Vec::new(),
            })
            .collect()
    }

    use mod_api::{AttributeModifier, AttributeOperation, Identifier, ModifierLifetime};

    /// A mob's movement speed is kept where its gameplay reads it: a mod's
    /// permanent modifier is saved, a transient one is not, both count at once.
    #[test]
    #[ignore = "daily"]
    fn a_mods_modifiers_on_a_mobs_speed_reach_its_gameplay() {
        use AttributeOperation::*;
        use ModifierLifetime::*;
        with_game(|game| {
            spawn(game);
            let speed = "minecraft:movement_speed";
            let zombie = living(game, 12);
            let (_, Some(base)) = read(game, 12, speed) else {
                panic!("a zombie has a movement speed");
            };
            assert!(zombie.add_attribute_modifier(
                game,
                speed,
                modifier("test:fast", 1.0, AddMultipliedBase),
                Permanent
            ));
            // The id is taken (vanilla throws).
            assert!(!zombie.add_attribute_modifier(
                game,
                speed,
                modifier("test:fast", 2.0, AddValue),
                Transient
            ));
            assert!(zombie.add_attribute_modifier(
                game,
                speed,
                modifier("test:boost", 0.5, AddMultipliedTotal),
                Transient
            ));
            let value = base * 2.0 * 1.5;
            assert_eq!(read(game, 12, speed), (Some(value), Some(base)));
            assert_eq!(game.mobs[0].attrs.speed, value);
            let saved = horizon_client::entity_nbt::attribute_entries(&game.mobs[0]);
            assert_eq!(saved_ids(saved.iter().map(|(_, e)| e)), ["test:fast"]);
            // Replacing takes the new lifetime: no longer saved.
            assert!(zombie.replace_attribute_modifier(
                game,
                speed,
                modifier("test:fast", 1.0, AddMultipliedBase),
                Transient
            ));
            let saved = horizon_client::entity_nbt::attribute_entries(&game.mobs[0]);
            assert!(saved_ids(saved.iter().map(|(_, e)| e)).is_empty());
            assert_eq!(read(game, 12, speed), (Some(value), Some(base)));
            let boost = Identifier::parse("test:boost").unwrap();
            assert!(zombie.remove_attribute_modifier(game, speed, &boost));
            assert!(!zombie.remove_attribute_modifier(game, speed, &boost));
            assert_eq!(read(game, 12, speed), (Some(base * 2.0), Some(base)));
            assert!(zombie.set_attribute_base(game, speed, 0.5));
            assert_eq!(read(game, 12, speed), (Some(1.0), Some(0.5)));
            assert_eq!(game.mobs[0].attrs.speed, 1.0);
        });
    }

    /// An attribute only the live map keeps keeps a mod's writes when the map
    /// is built again, and saves the permanent ones.
    #[test]
    #[ignore = "daily"]
    fn a_mods_writes_outlive_a_rebuilt_map() {
        use AttributeOperation::*;
        use ModifierLifetime::*;
        with_game(|game| {
            spawn(game);
            let gravity = "minecraft:gravity";
            let zombie = living(game, 12);
            assert!(zombie.set_attribute_base(game, gravity, 0.1));
            assert!(zombie.add_attribute_modifier(
                game,
                gravity,
                modifier("test:heavy", 1.0, AddMultipliedBase),
                Permanent
            ));
            assert!(zombie.add_attribute_modifier(
                game,
                gravity,
                modifier("test:float", -0.05, AddValue),
                Transient
            ));
            let both = Some((0.1 - 0.05) * 2.0);
            assert_eq!(read(game, 12, gravity), (both, Some(0.1)));
            game.mobs[0].extra.live.invalidate();
            assert_eq!(read(game, 12, gravity), (both, Some(0.1)));
            let mut list = Vec::new();
            game.mobs[0].extra.live.save_into(&mut list);
            assert_eq!(saved_ids(&list), ["test:heavy"]);
            let float = Identifier::parse("test:float").unwrap();
            assert!(zombie.remove_attribute_modifier(game, gravity, &float));
            game.mobs[0].extra.live.invalidate();
            assert_eq!(read(game, 12, gravity), (Some(0.2), Some(0.1)));
            // A change to `finalize`'s store rebuilds the map without losing them.
            horizon_client::mobs::finalize::set_base(
                &mut game.mobs[0],
                horizon_client::mobs::finalize::Attr::MaxHealth,
                30.0,
            );
            assert_eq!(read(game, 12, gravity), (Some(0.2), Some(0.1)));
            // The write is where `/attribute` reads it and the sync sends it.
            let key = game.motion.attributes.id(gravity).unwrap();
            let map = game.generic_attributes(Target::Mob(12), gravity).unwrap();
            assert_eq!(map.value(key).unwrap(), 0.2);
            let mut synced = Vec::new();
            let zombie_mob = game.mobs[0].clone();
            game.mob_attributes(
                &zombie_mob,
                &mut crate::game::lan_attributes::SpeedByType::default(),
                &mut synced,
            );
            let registry = key.registry_id();
            let sent = synced.iter().find(|s| s.attribute == registry).unwrap();
            assert_eq!(sent.base, 0.1);
            assert_eq!(
                sent.modifiers
                    .iter()
                    .map(|m| m.id.as_str())
                    .collect::<Vec<_>>(),
                ["test:heavy"]
            );
            // A mob's armor goes to its armor map, which its damage reads.
            assert!(zombie.set_attribute_base(game, "minecraft:armor", 10.0));
            assert_eq!(horizon_client::mobs::armor::values(&game.mobs[0]).0, 10.0);
            assert_eq!(read(game, 12, "minecraft:armor").1, Some(10.0));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_stand_the_dragon_and_a_player_take_writes() {
        use AttributeOperation::*;
        use ModifierLifetime::*;
        use horizon_client::motion::PlayerMotion;
        with_game(|game| {
            spawn(game);
            let health = "minecraft:max_health";
            let stand = living(game, 13);
            assert!(stand.set_attribute_base(game, health, 4.0));
            assert_eq!(read(game, 13, health), (Some(4.0), Some(4.0)));
            // Not an attribute of an armor stand; a non-finite amount.
            let damage = modifier("test:x", 1.0, AddValue);
            assert!(!stand.add_attribute_modifier(
                game,
                "minecraft:attack_damage",
                damage,
                Permanent
            ));
            assert!(!stand.add_attribute_modifier(
                game,
                health,
                modifier("test:x", f64::NAN, AddValue),
                Permanent
            ));
            // `/attribute` refuses the dragon, and nothing reads its map yet.
            let dragon = living(game, 14);
            assert!(!dragon.add_attribute_modifier(
                game,
                health,
                modifier("test:tough", 100.0, AddValue),
                Transient
            ));
            assert_eq!(read(game, 14, health), (Some(200.0), Some(200.0)));

            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            // The dragon owns its multipart IDs above 14; keep the fixture
            // player outside that range and reserve it against later spawns.
            game.next_entity = game.next_entity.max(100);
            crate::game::native_entity_refs::tests::bind_local_player(game, 100, 123);
            let player = living(game, 100);
            assert!(player.add_attribute_modifier(
                game,
                health,
                modifier("test:hearty", 0.5, AddMultipliedBase),
                Permanent
            ));
            assert_eq!(read(game, 100, health), (Some(30.0), Some(20.0)));
            let hearty = Identifier::parse("test:hearty").unwrap();
            assert!(player.remove_attribute_modifier(game, health, &hearty));
            assert_eq!(read(game, 100, health), (Some(20.0), Some(20.0)));
            // A stale handle writes nothing.
            let zombie = living(game, 12);
            game.mobs.clear();
            assert!(!zombie.set_attribute_base(game, "minecraft:gravity", 1.0));
        });
    }

    /// Plates every chestplate with +0.4 knockback resistance and no armor
    /// toughness (`item_attribute_modifier`).
    struct Plated;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Plated {
        fn item_attribute_modifier(&self, event: &mut mod_api::ItemAttributeModifierEvent<'_>) {
            if !mod_api::__private::item_key_id(event.stack.item())
                .unwrap()
                .ends_with("_chestplate")
            {
                return;
            }
            let id = |text: &str| Identifier::parse(text).unwrap();
            event.modifiers.add(
                id("minecraft:knockback_resistance"),
                modifier("test:plated", 0.4, AttributeOperation::AddValue),
                mod_api::EquipmentSlotGroup::Chest,
            );
            event
                .modifiers
                .remove_all_for(&id("minecraft:armor_toughness"));
        }
    }

    /// A mod's answer to `item_attribute_modifier` is what the zombie's
    /// gameplay reads: its knockback and its live map.
    #[test]
    #[ignore = "daily"]
    fn a_mods_item_modifiers_reach_the_wearer() {
        // The game's stack snapshots, which the hook needs, as its startup installs them;
        // without this the test passes only after another test in the process installed them.
        crate::game::api_values::install_stack_hooks();
        let mods = vec![mod_api::ModEntry::new("plated", Box::new(Plated))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                spawn(game);
                let chestplate = stack("minecraft:netherite_chestplate");
                game.mobs[0].set_equipment_stack(CHEST, chestplate);
                game.refresh_gear_knockback_resistance();
                game.tick_equipment_attributes();
                // Netherite's own 0.1, and the mod's 0.4.
                let resistance = game.mobs[0].gear_knockback_resistance.unwrap();
                assert!((resistance - 0.5).abs() < 1e-6, "{resistance}");
                let toughness = read(game, 12, "minecraft:armor_toughness");
                assert_eq!(toughness, (Some(0.0), Some(0.0)));
                // Netherite's 8 armor still counts, on the zombie's own 2.
                assert_eq!(read(game, 12, "minecraft:armor"), (Some(10.0), Some(2.0)));
            });
        });
    }
}
