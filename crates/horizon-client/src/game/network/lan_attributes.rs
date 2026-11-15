//! The attribute instances a server syncs for the living entities it tracks
//! (`ClientboundUpdateAttributesPacket`, lan/attributes.rs): the ones
//! vanilla's `AttributeMap` would have created by then, since only those are
//! sent, never the type's other defaults.
use super::Game;
use horizon_client::items::ItemStack;
use horizon_client::{
    effects,
    items::Items,
    lan::attributes::{add_modifier, instance, operation_named, syncable},
    mobs::{
        GearKnockback, Mob, Species,
        finalize::{self, Attr, Op},
    },
};
use horizon_protocol::packets::play::{AttributeModifier, AttributeSnapshot};
use std::sync::Arc;

/// `EquipmentSlotGroup.test`: whether a modifier's slot group covers a slot.
/// For one snapshot of the mobs, each type's movement speed: its synced
/// instance (`synced_attribute`) and the type's default base, both looked up
/// by name. The pointers are only compared while the snapshot holds the types.
#[derive(Default)]
pub(super) struct SpeedByType(
    horizon_client::fast_hash::FxHashMap<
        *const horizon_client::mobs::MobType,
        (Option<i32>, Option<f64>),
    >,
);

fn slot_in_group(group: &str, slot: &str) -> bool {
    group == slot
        || group == "any"
        || (group == "hand" && matches!(slot, "mainhand" | "offhand"))
        || (group == "armor" && matches!(slot, "feet" | "legs" | "chest" | "head" | "body"))
}

/// The hand and armor slots, mainhand to head, with their names.
const GEAR: [(usize, &str); 6] = {
    use horizon_items::equipment::{CHEST, FEET, HEAD, LEGS, MAINHAND, OFFHAND};
    [
        (MAINHAND, "mainhand"),
        (OFFHAND, "offhand"),
        (FEET, "feet"),
        (LEGS, "legs"),
        (CHEST, "chest"),
        (HEAD, "head"),
    ]
};

/// The knockback resistance modifiers of the stacks `worn` in `GEAR`'s
/// slots, where their slot groups cover the slot.
fn gear_knockback(items: &Items, worn: &[Option<&ItemStack>; 6]) -> Vec<(f64, Op)> {
    let mut modifiers = Vec::new();
    for ((_, slot), stack) in GEAR.iter().zip(worn) {
        let Some(stack) = stack else {
            continue;
        };
        for row in horizon_client::item_attributes::attribute_modifiers(stack, items).iter() {
            if row.attribute == "minecraft:knockback_resistance" && slot_in_group(&row.slot, slot) {
                let op = match operation_named(&row.operation) {
                    1 => Op::AddMultipliedBase,
                    2 => Op::AddMultipliedTotal,
                    _ => Op::AddValue,
                };
                modifiers.push((row.amount, op));
            }
        }
    }
    modifiers
}

/// `getAttributeValue(KNOCKBACK_RESISTANCE)` of a mob: its own value plus the
/// modifiers of the items it wears or holds, as the tick's
/// `refresh_gear_knockback_resistance` read them.
pub(super) fn mob_knockback_resistance(m: &Mob) -> f64 {
    let gear = m.extra.gear_knockback.as_deref();
    finalize::value_with(
        m,
        Attr::KnockbackResistance,
        gear.map_or(&[], |g| &g.modifiers),
    )
}

impl Game<'_> {
    /// Each mob's `gear_knockback_resistance`, for the pushes that read the
    /// mob alone. A worn or held stack is read again only when it changed
    /// (`ItemStack.matches`, as `collectEquipmentChanges` asks), so a tick
    /// with the same gear reads no item data and allocates nothing.
    pub(super) fn refresh_gear_knockback_resistance(&mut self) {
        let items = self.res.items();
        for m in &mut self.mobs {
            let worn = GEAR.map(|(slot, _)| m.extra.slot_stack(slot));
            if worn.iter().all(Option::is_none) {
                m.extra.gear_knockback = None;
                m.gear_knockback_resistance = None;
                continue;
            }
            let same = m.extra.gear_knockback.as_ref().is_some_and(|g| {
                g.worn.iter().zip(&worn).all(|pair| match pair {
                    (Some(was), Some(now)) => now.matches(was),
                    (was, now) => was.is_none() && now.is_none(),
                })
            });
            if !same {
                let gear = GearKnockback {
                    modifiers: gear_knockback(items, &worn),
                    worn: worn.map(|stack| stack.cloned()),
                };
                m.extra.gear_knockback = Some(Box::new(gear));
            }
            m.gear_knockback_resistance = Some(mob_knockback_resistance(m));
        }
    }

    /// The registry id of a client-syncable attribute the entity type has
    /// (`AttributeMap.getInstance` is null for the others), by name.
    fn synced_attribute(&self, entity: &str, name: &str) -> Option<i32> {
        let catalog = &self.motion.attributes;
        let id = catalog.id(name)?;
        (catalog.attribute(id).syncable && catalog.default_base(entity, id).is_some())
            .then(|| id.registry_id())
    }

    /// The type's base value of an attribute, as the instance starts.
    fn attribute_default(&self, entity: &str, name: &str) -> f64 {
        let catalog = &self.motion.attributes;
        catalog
            .id(name)
            .and_then(|id| catalog.default_base(entity, id))
            .unwrap_or(0.0)
    }

    /// A mob's synced instances: the live attribute map's (arismoko/horizon#941) — what
    /// finalizeSpawn set, `/attribute` changed, the save loaded and every
    /// worn or held stack granted, its base and each modifier exactly once,
    /// as the map holds them — plus movement speed (its first `aiStep` reads
    /// it, `LivingEntity.removeFrost`, and no stack granted one), the baby
    /// speed modifier and its effects' modifiers, which no map holds.
    pub(super) fn mob_attributes(
        &self,
        m: &Mob,
        speeds: &mut SpeedByType,
        out: &mut Vec<AttributeSnapshot>,
    ) {
        let kind = m.kind.id.as_str();
        // Movement speed is synced for nearly every mob, and read up to three
        // times below: its instance and the type's default are looked up by
        // name once per type and snapshot, its base once per mob.
        let (synced, default) = *speeds.0.entry(Arc::as_ptr(&m.kind)).or_insert_with(|| {
            (
                self.synced_attribute(kind, Attr::MovementSpeed.id()),
                finalize::default_value(m, Attr::MovementSpeed),
            )
        });
        let speed = synced.map(|a| {
            (
                a,
                finalize::base_value_with(m, Attr::MovementSpeed, || default),
            )
        });
        if let Some((a, base)) = speed {
            instance(out, a, base);
        }
        // The live map's syncable instances: `/attribute`'s writes, the
        // save's, a mod's modifiers and every worn or held stack's own
        // `attribute_modifiers`, components included. An immutable read
        // builds the map if this is its first use; the tick's equipment pass
        // keeps its stacks current. What the map holds is sent once, with the
        // base its instance has — never derived again beside it.
        let catalog = self.motion.attributes.clone();
        let _ = m.extra.live.read(
            &catalog,
            kind,
            Attr::MovementSpeed.id(),
            horizon_world::attribute_map::ValueKind::Computed,
            || {
                horizon_client::entity_nbt::attribute_entries(m)
                    .into_iter()
                    .map(|(_, e)| e)
                    .collect()
            },
        );
        if let Some(live) = m.extra.live.built() {
            for s in syncable(live) {
                let (attribute, base) = (s.attribute, s.base);
                *instance(out, attribute, base) = s;
            }
        }
        // AbstractHorse.randomizeAttributes rolls a horse's jump strength as
        // its base (kept in `vector[0]`), so the instance is synced: its
        // rider's client jumps it (`getCustomJump`).
        let rolled = matches!(
            m.species(),
            Species::Horse | Species::SkeletonHorse | Species::ZombieHorse
        );
        let jump_base = m
            .extra
            .attributes
            .as_deref()
            .is_some_and(|s| s.bases.iter().any(|(a, _)| *a == Attr::JumpStrength));
        if rolled
            && !jump_base
            && m.extra.vector[0] > 0.0
            && let Some(a) = self.synced_attribute(kind, Attr::JumpStrength.id())
        {
            instance(out, a, m.extra.vector[0]).base = m.extra.vector[0];
        }
        if let (Some((a, base)), Some((amount, Op::AddMultipliedBase))) =
            (speed, finalize::transient(m, Attr::MovementSpeed))
        {
            add_modifier(
                out,
                a,
                base,
                AttributeModifier {
                    id: "minecraft:baby".into(),
                    amount,
                    operation: 1,
                },
            );
        }
        // LivingEntity.setSprinting: SPEED_MODIFIER_SPRINTING (a dashing cat or ocelot).
        if let (true, Some((a, base))) = (m.extra.sprinting, speed) {
            add_modifier(
                out,
                a,
                base,
                AttributeModifier {
                    id: "minecraft:sprinting".into(),
                    amount: f64::from(0.3f32),
                    operation: 2,
                },
            );
        }
        let base = |name: &str| {
            Attr::ALL
                .into_iter()
                .find(|a| name.strip_prefix("minecraft:") == Some(a.name()))
                .map(|a| finalize::base_value(m, a))
        };
        // LivingEntity.onEffectAdded: MobEffect.addAttributeModifiers.
        for effect in &m.extra.effects {
            let (effect, amplifier) = (effect.id, effect.amplifier);
            let Some((attribute, id, amount, op)) =
                effects::effect_modifier(effect.trim_start_matches("minecraft:"))
            else {
                continue;
            };
            let Some(a) = self.synced_attribute(kind, attribute) else {
                continue;
            };
            let start = base(attribute).unwrap_or_else(|| self.attribute_default(kind, attribute));
            add_modifier(
                out,
                a,
                start,
                AttributeModifier {
                    id: id.to_owned(),
                    amount: amount * f64::from(amplifier + 1),
                    operation: operation_named(op),
                },
            );
        }
    }

    /// An armor stand's synced instances: movement speed (its first
    /// `aiStep`), and the live map's — its equipment's per-stack modifiers
    /// and what `/attribute` changed — built by the read if this is its
    /// first use.
    pub(super) fn stand_attributes(
        &self,
        live: &horizon_client::living_attributes::LivingAttributes,
    ) -> Vec<AttributeSnapshot> {
        const KIND: &str = "minecraft:armor_stand";
        let mut out = Vec::new();
        if let Some(a) = self.synced_attribute(KIND, "minecraft:movement_speed") {
            instance(
                &mut out,
                a,
                self.attribute_default(KIND, "minecraft:movement_speed"),
            );
        }
        let catalog = self.motion.attributes.clone();
        let _ = live.read(
            &catalog,
            KIND,
            "minecraft:movement_speed",
            horizon_world::attribute_map::ValueKind::Computed,
            Vec::new,
        );
        if let Some(map) = live.built() {
            for s in syncable(map) {
                let (attribute, base) = (s.attribute, s.base);
                *instance(&mut out, attribute, base) = s;
            }
        }
        out
    }

    /// The instances a server player's tick creates, and
    /// `ServerPlayer.updatePlayerAttributes`: a creative player reaches 0.5
    /// further to blocks and 2 to entities. Its client reads the reach from
    /// these synced modifiers. (The crouching player's waypoint range is
    /// server-side only: lan_waypoints.rs hides it.)
    pub(super) fn update_player_attributes(&mut self) {
        let creative = self.mode_id() == 1;
        let Some(p) = self.player.as_mut() else {
            return;
        };
        // LivingEntity.aiStep's removeFrost reads the movement speed instance
        // every tick, which creates it (and its first sync).
        if let Some(key) = p.attributes.key("minecraft:movement_speed") {
            let _ = p.attributes.get_instance(Some(key));
        }
        for (attribute, id, amount) in [
            (
                "minecraft:block_interaction_range",
                "minecraft:creative_mode_block_range",
                0.5,
            ),
            (
                "minecraft:entity_interaction_range",
                "minecraft:creative_mode_entity_range",
                2.0,
            ),
        ] {
            let Some(key) = p.attributes.key(attribute) else {
                continue;
            };
            // getAttribute creates the instance, a creative player's or not.
            if !matches!(p.attributes.get_instance(Some(key.clone())), Ok(Some(_))) {
                continue;
            }
            // addOrUpdateTransientModifier with the same modifier and
            // removeModifier of a missing one leave the instance clean.
            let has = p.attributes.has_modifier(Some(key), id).unwrap_or(false);
            if has != creative {
                let _ = p.set_modifier(attribute, id, amount, "add_value", creative);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::slot_in_group;

    #[test]
    fn slot_groups_cover_their_slots() {
        assert!(slot_in_group("head", "head"));
        assert!(slot_in_group("armor", "chest"));
        assert!(!slot_in_group("armor", "mainhand"));
        assert!(slot_in_group("hand", "offhand"));
        assert!(slot_in_group("any", "feet"));
        assert!(!slot_in_group("mainhand", "offhand"));
    }
}
