//! Borrowed live component queries for decoded ingredient predicates.
use super::component_values::damage_resistant_equal;
use super::*;

/// Visit present component names without serializing or allocating.
pub fn visit_components(stack: &ItemStack, mut visit: impl FnMut(&str)) {
    for id in MODELLED {
        if stack.carries_component(None, id) {
            visit(id);
        }
    }
    for id in [
        "minecraft:item_name",
        "minecraft:rarity",
        "minecraft:tooltip_display",
    ] {
        if stack.carries_component(None, id) {
            visit(id);
        }
    }
    if let Some(data) = stack.data() {
        for (id, _) in &data.other {
            if !id.starts_with('!') && !MODELLED.contains(&id.as_str()) {
                visit(id);
            }
        }
    }
}

/// Whether a patch explicitly removed this component, without building a prefixed key.
pub fn removed(stack: &ItemStack, id: &str) -> bool {
    !stack.carries_component(None, id)
        && stack.data().is_some_and(|data| {
            data.other
                .iter()
                .any(|(key, _)| key.strip_prefix('!') == Some(id))
        })
}

/// Equality in the decoded representation used by the shared component codecs.
/// Presence is checked by the caller, so empty decoded enchantment maps compare equal.
pub fn component_equal(id: &str, a: &ItemStack, b: &ItemStack) -> bool {
    match id {
        "minecraft:waxed" | "minecraft:intangible_projectile" | "minecraft:glider" => {
            matches!(a.data().and_then(|d| d.other(id)), Some(Tag::Compound(_)))
                && matches!(b.data().and_then(|d| d.other(id)), Some(Tag::Compound(_)))
        }

        "minecraft:cat/sound_variant"
        | "minecraft:cat/variant"
        | "minecraft:wolf/sound_variant"
        | "minecraft:wolf/variant" => {
            let a = a.data().and_then(|d| d.other(id));
            let b = b.data().and_then(|d| d.other(id));
            match (a, b) {
                (Some(a), Some(b)) => {
                    match (
                        crate::properties::resolvable::identifier(a),
                        crate::properties::resolvable::identifier(b),
                    ) {
                        (Some(a), Some(b)) => a == b,
                        _ => false,
                    }
                }
                (None, None) => true,
                _ => false,
            }
        }

        "minecraft:container_loot" => {
            fn fields(stack: &ItemStack) -> Option<(&Tag, i64)> {
                stack
                    .data()
                    .and_then(|d| d.other("minecraft:container_loot"))
                    .and_then(|tag| container_loot_fields(tag).ok())
            }
            match (fields(a), fields(b)) {
                (Some((table_a, seed_a)), Some((table_b, seed_b))) => {
                    seed_a == seed_b && identifier_equal(Some(table_a), Some(table_b))
                }
                _ => false,
            }
        }

        "minecraft:tooltip_display" => tooltip_display_equal(a, b),
        "minecraft:lodestone_tracker" => lodestone_tracker_equal(
            a.data().and_then(|d| d.other(id)),
            b.data().and_then(|d| d.other(id)),
        ),
        "minecraft:recipes" => recipe_keys_equal(
            a.data().and_then(|d| d.other(id)),
            b.data().and_then(|d| d.other(id)),
        ),
        "minecraft:mob_visibility" => mob_visibility_equal(
            a.data().and_then(|d| d.other(id)),
            b.data().and_then(|d| d.other(id)),
        ),
        "minecraft:cooking_fuel" => super::cooking_fuel::equal(
            a.data().and_then(|d| d.other(id)),
            b.data().and_then(|d| d.other(id)),
        ),
        "minecraft:brewing_fuel" => super::brewing_fuel::equal(
            a.data().and_then(|d| d.other(id)),
            b.data().and_then(|d| d.other(id)),
        ),
        "minecraft:compostable" => {
            let a = a
                .data()
                .and_then(|d| d.other(id))
                .and_then(|t| compostable_layers(t).ok());
            let b = b
                .data()
                .and_then(|d| d.other(id))
                .and_then(|t| compostable_layers(t).ok());
            matches!((a,b),(Some(a),Some(b)) if a==b)
        }

        "minecraft:weapon" => {
            let weapon = |stack: &ItemStack| {
                stack
                    .data()
                    .and_then(|data| data.other(id))
                    .and_then(|tag| crate::properties::Weapon::from_tag(tag).ok())
            };
            match (weapon(a), weapon(b)) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            }
        }
        "minecraft:attack_animation" => {
            use crate::properties::swing_animation::SwingAnimation;
            let animation = |stack: &ItemStack| {
                stack
                    .data()
                    .and_then(|data| data.other(id))
                    .and_then(SwingAnimation::decode)
            };
            animation(a).is_some_and(|expected| animation(b) == Some(expected))
        }
        "minecraft:attack_range" => {
            let decoded = |stack: &ItemStack| {
                stack
                    .data()
                    .and_then(|data| data.other(id))
                    .and_then(crate::properties::attack_range::decode)
            };
            match (decoded(a), decoded(b)) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            }
        }
        // `Unit.CODEC` has a single value; the caller already checked effective presence.
        "minecraft:unbreakable" => true,
        "minecraft:damage" => a.damage == b.damage,
        "minecraft:map_id" => map_id(a) == map_id(b),
        "minecraft:repair_cost" => {
            a.data().map_or(0, |d| d.repair_cost) == b.data().map_or(0, |d| d.repair_cost)
        }
        "minecraft:dyed_color" => dyed_color(a).is_some_and(|color| Some(color) == dyed_color(b)),
        "minecraft:custom_model_data" => {
            match (
                a.data().and_then(|data| data.other(id)),
                b.data().and_then(|data| data.other(id)),
            ) {
                (Some(a), Some(b)) => custom_model_data::equal(a, b),
                _ => false,
            }
        }
        "minecraft:enchantments" => levels_equal(
            a.data().map_or(&[], |d| d.enchantments.as_slice()),
            b.data().map_or(&[], |d| d.enchantments.as_slice()),
        ),
        "minecraft:stored_enchantments" => levels_equal(
            a.data().map_or(&[], |d| d.stored_enchantments.as_slice()),
            b.data().map_or(&[], |d| d.stored_enchantments.as_slice()),
        ),
        "minecraft:enchantment_glint_override" => {
            a.data().and_then(|d| d.other(id)).and_then(component_bool)
                == b.data().and_then(|d| d.other(id)).and_then(component_bool)
        }
        "minecraft:food" => match (
            a.data().and_then(|d| d.other(id)).map(food_properties),
            b.data().and_then(|d| d.other(id)).map(food_properties),
        ) {
            (Some(Ok(a)), Some(Ok(b))) => a == b,
            _ => false,
        },
        "minecraft:custom_name" => {
            a.data().and_then(|d| d.custom_name.as_ref())
                == b.data().and_then(|d| d.custom_name.as_ref())
        }
        "minecraft:max_damage" | "minecraft:max_stack_size" => {
            a.data()
                .and_then(|d| d.other(id))
                .and_then(crate::properties::component_int)
                == b.data()
                    .and_then(|d| d.other(id))
                    .and_then(crate::properties::component_int)
        }
        // Enchantable.CODEC ignores other record fields and converts boxed numbers.
        "minecraft:enchantable" => {
            let value = |stack: &ItemStack| {
                stack
                    .data()
                    .and_then(|data| data.other(id))
                    .and_then(|tag| tag.get("value"))
                    .and_then(crate::properties::component_int)
                    .filter(|value| *value > 0)
            };
            value(a) == value(b)
        }
        "minecraft:potion_duration_scale" => {
            // `Float.equals` uses floatToIntBits: signed zeros differ, all NaNs agree.
            let bits = |stack: &ItemStack| {
                stack
                    .data()
                    .and_then(|data| data.other(id))
                    .and_then(crate::properties::component_float)
                    .map(|value| {
                        if value.is_nan() {
                            0x7fc0_0000
                        } else {
                            value.to_bits()
                        }
                    })
            };
            bits(a) == bits(b)
        }
        "minecraft:minimum_attack_charge" => {
            let bits = |stack: &ItemStack| {
                stack
                    .data()
                    .and_then(|data| data.other(id))
                    .and_then(crate::properties::component_float)
                    .map(f32::to_bits)
            };
            bits(a) == bits(b)
        }
        // `VillagerFood.equals`: only the decoded nutrition participates.
        "minecraft:villager_food" => {
            let nutrition = |stack: &ItemStack| {
                stack
                    .data()
                    .and_then(|data| data.other(id))
                    .and_then(|value| value.get("nutrition"))
                    .and_then(crate::properties::component_int)
                    .filter(|value| *value > 0)
            };
            nutrition(a) == nutrition(b)
        }
        "minecraft:block_entity_data" => block_entity_equal(
            a.data().and_then(|d| d.other(id)),
            b.data().and_then(|d| d.other(id)),
        ),
        "minecraft:item_name" => item_name_equal(a, b),
        "minecraft:item_model" | "minecraft:tooltip_style" => identifier_equal(
            a.data().and_then(|d| d.other(id)),
            b.data().and_then(|d| d.other(id)),
        ),
        "minecraft:use_cooldown" => {
            fn fields(stack: &ItemStack) -> Option<(f32, Option<&Tag>)> {
                stack
                    .data()
                    .and_then(|d| d.other("minecraft:use_cooldown"))
                    .and_then(crate::cooldowns::UseCooldown::fields_from_tag)
            }
            match (fields(a), fields(b)) {
                (Some((a, ga)), Some((b, gb))) => {
                    a.to_bits() == b.to_bits()
                        && match (ga, gb) {
                            (None, None) => true,
                            (Some(a), Some(b)) => identifier_equal(Some(a), Some(b)),
                            _ => false,
                        }
                }
                _ => false,
            }
        }
        "minecraft:damage_resistant" => damage_resistant_equal(
            a.data().and_then(|d| d.other(id)),
            b.data().and_then(|d| d.other(id)),
        ),
        "minecraft:use_effects" => {
            match (
                a.data().and_then(|d| d.other(id)).map(use_effects),
                b.data().and_then(|d| d.other(id)).map(use_effects),
            ) {
                (Some(Ok(a)), Some(Ok(b))) => a == b,
                _ => false,
            }
        }
        "minecraft:base_color" => {
            a.data().and_then(|d| d.base_color.as_deref())
                == b.data().and_then(|d| d.base_color.as_deref())
        }
        "minecraft:note_block_sound" => identifier_equal(
            a.data().and_then(|d| d.other(id)),
            b.data().and_then(|d| d.other(id)),
        ),
        "minecraft:rarity" => rarity(a) == rarity(b),
        "minecraft:ominous_bottle_amplifier" => ominous_amplifier(a) == ominous_amplifier(b),
        // DyeColor.CODEC's canonical NbtOps value is its exact serialized name.
        // Live stacks retain these colors in `other`; borrow UTF-16 without palette scans.
        "minecraft:tropical_fish/base_color"
        | "minecraft:tropical_fish/pattern_color"
        | "minecraft:cushion/color"
        | "minecraft:dye"
        | "minecraft:wolf/collar"
        | "minecraft:cat/collar"
        | "minecraft:shulker/color"
        | "minecraft:sheep/color" => match (
            a.data().and_then(|d| d.other(id)),
            b.data().and_then(|d| d.other(id)),
        ) {
            (Some(Tag::String(a)), Some(Tag::String(b))) => a == b,
            _ => false,
        },
        _ => stored_component_equal(id, a, b),
    }
}

/// `MapId.CODEC`: compare the borrowed boxed-number value or the modelled id.
fn map_id(stack: &ItemStack) -> Option<i32> {
    let data = stack.data()?;
    match data.other("minecraft:map_id") {
        Some(value) => crate::properties::component_int(value),
        None => data.map_id,
    }
}

fn dyed_color(stack: &ItemStack) -> Option<i32> {
    let data = stack.data()?;
    match data.other("minecraft:dyed_color") {
        Some(raw) => horizon_core::color::rgb_color_from_tag(raw),
        None => data.dyed_color,
    }
}

/// `Compostable.CODEC` / `ResolvableInt.CODEC`: required layers, either any Java
/// int or a resource key. References are keys, not registry lookups.
fn compostable_layers(value: &Tag) -> Result<crate::properties::resolvable::Int<'_>, &'static str> {
    if !matches!(value, Tag::Compound(_)) {
        return Err("Not a map");
    }
    let layers = value.get("layers").ok_or("Missing layers")?;
    crate::properties::resolvable::int(layers)
        .ok_or("layers: expected a number or resource location")
}

/// Validate without materializing a normalized payload.
pub fn validate_compostable(value: &Tag) -> Result<(), String> {
    compostable_layers(value).map(|_| ()).map_err(str::to_owned)
}

/// Normalize only the persistent codec fields; unknown record fields are ignored.
pub fn decoded_compostable(value: &Tag) -> Result<Tag, String> {
    let layers = compostable_layers(value).map_err(str::to_owned)?.to_tag();
    Ok(Tag::compound([("layers", layers)]))
}
/// `UseEffects.CODEC`: strict optional fields, ignored unknown fields and NbtOps numbers.
/// The float is returned as bits for Java record equality (including signed zero).
pub fn use_effects(value: &Tag) -> Result<(bool, bool, u32), &'static str> {
    if !matches!(value, Tag::Compound(_)) {
        return Err("Not a map");
    }
    let can_sprint = value
        .get("can_sprint")
        .filter(|v| !matches!(v, Tag::End))
        .map(component_bool)
        .unwrap_or(Some(false))
        .ok_or("can_sprint: Not a number")?;
    let interact_vibrations = value
        .get("interact_vibrations")
        .filter(|v| !matches!(v, Tag::End))
        .map(component_bool)
        .unwrap_or(Some(true))
        .ok_or("interact_vibrations: Not a number")?;
    let speed = value
        .get("speed_multiplier")
        .filter(|v| !matches!(v, Tag::End))
        .map(crate::properties::component_float)
        .unwrap_or(Some(0.2))
        .ok_or("speed_multiplier: Not a number")?;
    // DFU Codec.floatRange uses checkRange/Float.compareTo, not primitive `<`.
    // Thus -0.0 is below the lower bound, and NaN is above the upper bound.
    if speed.is_sign_negative() || !(0.0..=1.0).contains(&speed) {
        return Err("speed_multiplier: Value outside of range [0:1]");
    }
    Ok((can_sprint, interact_vibrations, speed.to_bits()))
}

/// Exact stored-value comparison, including the fields removed from `other` by decoding.
pub fn stored_component_equal(id: &str, a: &ItemStack, b: &ItemStack) -> bool {
    if id == "minecraft:damage" {
        return a.damage == b.damage;
    }
    if !MODELLED.contains(&id) {
        return a.data().and_then(|d| d.other(id)) == b.data().and_then(|d| d.other(id));
    }
    let raw_a = a.data().and_then(|data| {
        data.other
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, value)| value)
    });
    let raw_b = b.data().and_then(|data| {
        data.other
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, value)| value)
    });
    // Empty/malformed retained values are not interchangeable with a populated modelled field.
    if raw_a.is_some() || raw_b.is_some() {
        return raw_a == raw_b;
    }
    let empty = StackData::default();
    let a = a.data().unwrap_or(&empty);
    let b = b.data().unwrap_or(&empty);
    match id {
        "minecraft:potion_contents" => {
            a.potion == b.potion
                && a.potion_extra.len() == b.potion_extra.len()
                && a.potion_extra
                    .iter()
                    .all(|entry| b.potion_extra.contains(entry))
        }
        "minecraft:bundle_contents" => stacks_equal(&a.bundle_contents, &b.bundle_contents),
        "minecraft:charged_projectiles" => stacks_equal(&a.charged, &b.charged),
        "minecraft:container" => {
            a.container.len() == b.container.len()
                && a.container
                    .iter()
                    .zip(&b.container)
                    .all(|((slot, stack), (other, value))| slot == other && stack.matches(value))
        }
        "minecraft:trim" => a.trim == b.trim,
        "minecraft:fireworks" => a.fireworks == b.fireworks,
        "minecraft:firework_explosion" => a.firework_explosion == b.firework_explosion,
        "minecraft:map_id" => a.map_id == b.map_id,
        "minecraft:base_color" => a.base_color == b.base_color,
        "minecraft:dyed_color" => a.dyed_color == b.dyed_color,
        "minecraft:ominous_bottle_amplifier" => a.ominous_amplifier == b.ominous_amplifier,
        "minecraft:map_post_processing" => a.map_post_processing == b.map_post_processing,
        "minecraft:writable_book_content" => a.writable_pages == b.writable_pages,
        "minecraft:written_book_content" => a.written == b.written,
        "minecraft:suspicious_stew_effects" => a.stew_effects == b.stew_effects,
        "minecraft:pot_decorations" => match (&a.pot_decorations, &b.pot_decorations) {
            (Some(a), Some(b)) => a.iter().zip(b.iter()).all(|(a, b)| match (a, b) {
                (Some(a), Some(b)) => a.matches(b),
                (None, None) => true,
                _ => false,
            }),
            (None, None) => true,
            _ => false,
        },
        "minecraft:banner_patterns" => banner_equal(a, b),
        "minecraft:consumable" => {
            a.consumable.as_ref().map(|value| &value.raw)
                == b.consumable.as_ref().map(|value| &value.raw)
        }
        "minecraft:kinetic_weapon" => {
            a.kinetic_weapon.as_ref().map(|value| value.raw())
                == b.kinetic_weapon.as_ref().map(|value| value.raw())
        }
        // These have shared decoders; keep this helper complete for direct users too.
        "minecraft:repair_cost" => a.repair_cost == b.repair_cost,
        "minecraft:enchantments" => levels_equal(&a.enchantments, &b.enchantments),
        "minecraft:stored_enchantments" => {
            levels_equal(&a.stored_enchantments, &b.stored_enchantments)
        }
        "minecraft:custom_name" => a.custom_name == b.custom_name,
        _ => unreachable!("every modelled component has a borrowed comparison"),
    }
}
fn stacks_equal(a: &[ItemStack], b: &[ItemStack]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.matches(b))
}
fn banner_equal(a: &StackData, b: &StackData) -> bool {
    match (a.ominous_banner, b.ominous_banner) {
        (true, true) => true,
        (false, false) => a.banner_patterns == b.banner_patterns,
        (true, false) => ominous_layers(&b.banner_patterns),
        (false, true) => ominous_layers(&a.banner_patterns),
    }
}
fn ominous_layers(layers: &[crate::banner_layer::BannerLayer]) -> bool {
    layers.len() == OMINOUS_LAYERS.len()
        && layers
            .iter()
            .zip(OMINOUS_LAYERS)
            .all(|(layer, (pattern, color))| {
                layer
                    .pattern
                    .strip_prefix("minecraft:")
                    .unwrap_or(&layer.pattern)
                    == pattern
                    && layer.color.as_ref() == color
            })
}

fn java_is(value: &JavaString, text: &str) -> bool {
    value.0.iter().copied().eq(text.encode_utf16())
}
fn implicit_ominous_name(stack: &ItemStack) -> bool {
    stack
        .data()
        .is_some_and(|data| data.ominous_banner && data.other("minecraft:item_name").is_none())
}
fn is_ominous_name(tag: Option<&Tag>) -> bool {
    let Some(Tag::Compound(map)) = tag else {
        return false;
    };
    let mut translate = false;
    map.iter().all(|(key, value)| {
        if java_is(key, "translate") {
            translate = matches!(value, Tag::String(value) if java_is(value, "block.minecraft.ominous_banner"));
            translate
        } else if java_is(key, "type") {
            matches!(value, Tag::String(value) if java_is(value, "translatable"))
        } else if java_is(key, "extra") || java_is(key, "with") {
            matches!(value, Tag::List(values) if values.is_empty())
        } else { false }
    }) && translate
}
fn item_name_equal(a: &ItemStack, b: &ItemStack) -> bool {
    let a_name = a.data().and_then(|data| data.other("minecraft:item_name"));
    let b_name = b.data().and_then(|data| data.other("minecraft:item_name"));
    match (implicit_ominous_name(a), implicit_ominous_name(b)) {
        (true, true) => true,
        (true, false) => is_ominous_name(b_name),
        (false, true) => is_ominous_name(a_name),
        (false, false) => a_name.map(text_value) == b_name.map(text_value),
    }
}
fn rarity(stack: &ItemStack) -> Option<u8> {
    let data = stack.data()?;
    match data.other("minecraft:rarity") {
        Some(Tag::String(value)) => ["common", "uncommon", "rare", "epic"]
            .iter()
            .position(|text| java_is(value, text))
            .map(|index| index as u8),
        Some(_) => None,
        None => data.ominous_banner.then_some(1),
    }
}

/// `OminousBottleAmplifier.CODEC`: borrow the effective raw override or modelled value.
fn ominous_amplifier(stack: &ItemStack) -> Option<i32> {
    let data = stack.data()?;
    let value = match data.other("minecraft:ominous_bottle_amplifier") {
        Some(raw) => crate::properties::component_int(raw)?,
        None => i32::from(data.ominous_amplifier?),
    };
    (0..=4).contains(&value).then_some(value)
}
/// `Identifier.bySeparator`/`equals`: default namespaces, borrowing Java UTF-16.
fn identifier_equal(a: Option<&Tag>, b: Option<&Tag>) -> bool {
    let (Some(Tag::String(a)), Some(Tag::String(b))) = (a, b) else {
        return false;
    };
    fn units(value: &JavaString) -> impl Iterator<Item = u16> + '_ {
        let (prefix, path) = match value.0.iter().position(|unit| *unit == u16::from(b':')) {
            Some(0) => ("minecraft:", &value.0[1..]),
            None => ("minecraft:", value.0.as_slice()),
            Some(_) => ("", value.0.as_slice()),
        };
        prefix.encode_utf16().chain(path.iter().copied())
    }
    units(a).eq(units(b))
}

/// `MobVisibility.CODEC`: required holder set and inclusive boxed-float range.
/// Registry references are resolved against the load context, never a built-in whitelist.
pub fn decode_mob_visibility(
    value: &Tag,
    registry: &horizon_client_core::registries::Registries,
) -> Result<Tag, String> {
    let parsed = MobVisibility::parse(value)
        .ok_or("Invalid mob_visibility fields or visibility outside [0;10]")?;
    let targets =
        super::component_values::holder_set(parsed.targets, "minecraft:entity_type", registry)?;
    Ok(Tag::compound([
        ("targeting_entity_types", targets),
        ("visibility", Tag::Float(parsed.visibility.to_bits())),
    ]))
}

struct MobVisibility<'a> {
    targets: &'a Tag,
    visibility: f32,
}

impl<'a> MobVisibility<'a> {
    /// Borrow live persistent storage, accepting compact singleton holder sets.
    fn parse(value: &'a Tag) -> Option<Self> {
        let Tag::Compound(_) = value else { return None };
        let targets = value.get("targeting_entity_types")?;
        let visibility = match value.get("visibility")? {
            Tag::Byte(v) => f32::from(*v),
            Tag::Short(v) => f32::from(*v),
            Tag::Int(v) => *v as f32,
            Tag::Long(v) => *v as f32,
            Tag::Float(v) => f32::from_bits(*v),
            Tag::Double(v) => f64::from_bits(*v) as f32,
            _ => return None,
        };
        // Float.compareTo distinguishes negative zero and orders NaN above 10.
        if !visibility.is_finite() || visibility.is_sign_negative() || visibility > 10.0 {
            return None;
        }
        Some(Self {
            targets,
            visibility,
        })
    }
}

fn mob_visibility_equal(a: Option<&Tag>, b: Option<&Tag>) -> bool {
    let (Some(a), Some(b)) = (
        a.and_then(MobVisibility::parse),
        b.and_then(MobVisibility::parse),
    ) else {
        return false;
    };
    if a.visibility.to_bits() != b.visibility.to_bits() {
        return false;
    }
    super::component_values::holder_set_equal(a.targets, b.targets)
}

fn block_entity_equal(a: Option<&Tag>, b: Option<&Tag>) -> bool {
    let (Some(Tag::Compound(a)), Some(Tag::Compound(b))) = (a, b) else {
        return a == b;
    };
    if a.len() != b.len() {
        return false;
    }
    a.iter().all(|(key, value)| {
        let Some(other) = b.get(key) else {
            return false;
        };
        if key.0.iter().copied().eq("id".encode_utf16()) {
            let (Tag::String(a), Tag::String(b)) = (value, other) else {
                return false;
            };
            let a_prefix = if a.0.contains(&u16::from(b':')) {
                ""
            } else {
                "minecraft:"
            };
            let b_prefix = if b.0.contains(&u16::from(b':')) {
                ""
            } else {
                "minecraft:"
            };
            a_prefix
                .encode_utf16()
                .chain(a.0.iter().copied())
                .eq(b_prefix.encode_utf16().chain(b.0.iter().copied()))
        } else {
            value == other
        }
    })
}

fn levels_equal(a: &[(Arc<str>, u32)], b: &[(Arc<str>, u32)]) -> bool {
    a.len() == b.len() && a.iter().all(|entry| b.contains(entry))
}
// Borrow the same plain-literal normalization as collapsed_text, without cloning a tag.
fn text_value(tag: &Tag) -> &Tag {
    let Tag::Compound(map) = tag else { return tag };
    let key_is = |key: &JavaString, value: &str| key.0.iter().copied().eq(value.encode_utf16());
    let Some((_, text @ Tag::String(_))) = map.iter().find(|(key, _)| key_is(key, "text")) else {
        return tag;
    };
    if map.iter().all(|(key, value)| {
        key_is(key, "text")
            || key_is(key, "type") && matches!(value, Tag::String(value) if key_is(value, "text"))
            || key_is(key, "extra") && matches!(value, Tag::List(values) if values.is_empty())
    }) {
        text
    } else {
        tag
    }
}

fn recipe_keys_equal(a: Option<&Tag>, b: Option<&Tag>) -> bool {
    fn keys(value: Option<&Tag>) -> Option<&[Tag]> {
        match value? {
            Tag::List(values) => Some(values),
            Tag::ByteArray(values) if values.is_empty() => Some(&[]),
            Tag::IntArray(values) if values.is_empty() => Some(&[]),
            Tag::LongArray(values) if values.is_empty() => Some(&[]),
            _ => None,
        }
    }
    let (Some(a), Some(b)) = (keys(a), keys(b)) else {
        return false;
    };
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(a, b)| identifier_equal(Some(a), Some(b)))
}

/// Read the record without cloning its payload. Unknown record fields are ignored;
/// present malformed optional fields are errors, not their absent-field defaults.
pub fn lodestone_tracker(value: &Tag) -> Result<LodestoneTracker<'_>, &'static str> {
    // Avoid even a first-use allocation of Tag::get's thread-local UTF-16 key.
    fn field<'a>(value: &'a Tag, name: &str) -> Option<&'a Tag> {
        let Tag::Compound(fields) = value else {
            return None;
        };
        fields
            .iter()
            .find_map(|(key, value)| java_is(key, name).then_some(value))
    }
    if !matches!(value, Tag::Compound(_)) {
        return Err("Not a map");
    }
    let target = match field(value, "target") {
        None | Some(Tag::End) => None,
        Some(target @ Tag::Compound(_)) => {
            let dimension = field(target, "dimension").ok_or("Missing target dimension")?;
            if !matches!(dimension, Tag::String(_)) {
                return Err("Target dimension is not a string");
            }
            let pos = block_pos(field(target, "pos").ok_or("Missing target pos")?)?;
            Some((dimension, pos))
        }
        Some(_) => return Err("Target is not a map"),
    };
    let tracked = match field(value, "tracked") {
        None | Some(Tag::End) => true,
        Some(value) => horizon_world::datafix::dynamic::as_bool_opt(Some(value))
            .ok_or("Tracked is not a number")?,
    };
    Ok(LodestoneTracker { target, tracked })
}

/// `BlockPos.CODEC`/`NbtOps.getIntStream`: exactly three boxed-number int values,
/// including all collection-tag forms, with no world-coordinate bounds.
fn block_pos(value: &Tag) -> Result<[i32; 3], &'static str> {
    fn three<T>(values: &[T]) -> Result<&[T; 3], &'static str> {
        values
            .try_into()
            .map_err(|_| "Input is not a list of 3 ints")
    }
    Ok(match value {
        Tag::IntArray(values) => *three(values)?,
        Tag::ByteArray(values) => three(values)?.map(|value| i32::from(value as i8)),
        Tag::LongArray(values) => three(values)?.map(|value| value as i32),
        Tag::List(values) => {
            let values = three(values)?;
            let mut pos = [0; 3];
            for (slot, value) in pos.iter_mut().zip(values) {
                *slot = horizon_world::datafix::dynamic::as_number(value)
                    .ok_or("Pos contains a non-number")?
                    .int_value();
            }
            pos
        }
        _ => return Err("Pos is not a list"),
    })
}

fn lodestone_tracker_equal(a: Option<&Tag>, b: Option<&Tag>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return false;
    };
    let (Ok(a), Ok(b)) = (lodestone_tracker(a), lodestone_tracker(b)) else {
        return false;
    };
    a.tracked == b.tracked
        && match (a.target, b.target) {
            (None, None) => true,
            (Some((a, pos)), Some((b, other))) => {
                pos == other && identifier_equal(Some(a), Some(b))
            }
            _ => false,
        }
}

fn tooltip_display_equal(a: &ItemStack, b: &ItemStack) -> bool {
    use crate::properties::tooltip_display::TooltipDisplay;
    fn raw(stack: &ItemStack) -> Option<&Tag> {
        stack
            .data()
            .and_then(|data| data.other("minecraft:tooltip_display"))
    }
    match (raw(a), raw(b)) {
        (Some(a), Some(b)) => match (TooltipDisplay::parse(a), TooltipDisplay::parse(b)) {
            (Some(a), Some(b)) => a.equal(b),
            _ => false,
        },
        (Some(value), None) => {
            b.data().is_some_and(|data| data.ominous_banner)
                && TooltipDisplay::parse(value).is_some_and(|value| value.is_ominous())
        }
        (None, Some(value)) => {
            a.data().is_some_and(|data| data.ominous_banner)
                && TooltipDisplay::parse(value).is_some_and(|value| value.is_ominous())
        }
        (None, None) => {
            a.data().is_some_and(|data| data.ominous_banner)
                == b.data().is_some_and(|data| data.ominous_banner)
        }
    }
}

pub struct LodestoneTracker<'a> {
    pub target: Option<(&'a Tag, [i32; 3])>,
    pub tracked: bool,
}

/// Borrow `SeededContainerLoot.CODEC`'s record fields, ignoring unknown fields.
/// The caller validates/normalizes the required `Identifier.CODEC` string at load;
/// live comparisons use `identifier_equal` without allocating.
pub fn container_loot_fields(value: &Tag) -> Result<(&Tag, i64), &'static str> {
    if !matches!(value, Tag::Compound(_)) {
        return Err("Not a map");
    }
    let table = value.get("loot_table").ok_or("Missing loot_table")?;
    if !matches!(table, Tag::String(_)) {
        return Err("loot_table: Not a string");
    }
    let seed = match value.get("seed") {
        None | Some(Tag::End) => 0,
        Some(seed) => horizon_world::datafix::dynamic::as_number(seed)
            .ok_or("seed: Not a number")?
            .long_value(),
    };
    Ok((table, seed))
}

#[cfg(test)]
mod tests {
    #[test]
    fn pet_variant_equality_borrows_normalized_registry_keys() {
        for component in [
            "minecraft:cat/sound_variant",
            "minecraft:cat/variant",
            "minecraft:wolf/sound_variant",
            "minecraft:wolf/variant",
        ] {
            let mut a = ItemStack::new("minecraft:stone", 1);
            let mut b = ItemStack::new("minecraft:stone", 1);
            a.data_mut()
                .set_other(component, Some(Tag::String("minecraft:classic".into())));
            for alias in ["classic", ":classic", "minecraft:classic"] {
                b.data_mut()
                    .set_other(component, Some(Tag::String(alias.into())));
                assert!(component_equal(component, &a, &b));
            }
            b.data_mut()
                .set_other(component, Some(Tag::String("example:classic".into())));
            assert!(!component_equal(component, &a, &b));
            a.data_mut()
                .set_other(component, Some(Tag::String("example:classic".into())));
            assert!(component_equal(component, &a, &b));
            a.data_mut()
                .set_other(component, Some(Tag::String("Classic".into())));
            b.data_mut()
                .set_other(component, Some(Tag::String("Classic".into())));
            assert!(!component_equal(component, &a, &b));
            b.data_mut().set_other(component, None);
            assert!(!component_equal(component, &a, &b));
        }
    }

    use super::*;
    #[test]
    fn decoded_component_equality_borrows_maps_and_plain_text() {
        let mut a = ItemStack::new("minecraft:stone", 1);
        let mut b = ItemStack::new("minecraft:stone", 1);
        a.data_mut().enchantments = vec![
            ("minecraft:sharpness".into(), 1),
            ("minecraft:unbreaking".into(), 2),
        ];
        b.data_mut().enchantments = vec![
            ("minecraft:unbreaking".into(), 2),
            ("minecraft:sharpness".into(), 1),
        ];
        assert!(component_equal("minecraft:enchantments", &a, &b));
        a.data_mut()
            .set_other("minecraft:item_name", Some(Tag::String("Named".into())));
        b.data_mut().set_other(
            "minecraft:item_name",
            Some(Tag::compound([("text", Tag::String("Named".into()))])),
        );
        assert!(component_equal("minecraft:item_name", &a, &b));
        b.data_mut().enchantments[0].1 = 3;
        assert!(!component_equal("minecraft:enchantments", &a, &b));
    }

    #[test]
    fn decoded_base_color_equality_uses_the_dedicated_field() {
        let mut a = ItemStack::new("minecraft:shield", 1);
        let mut b = ItemStack::new("minecraft:shield", 1);
        a.data_mut().base_color = Some("red".into());
        b.data_mut().base_color = Some("red".into());
        assert!(component_equal("minecraft:base_color", &a, &b));
        b.data_mut().base_color = Some("blue".into());
        assert!(!component_equal("minecraft:base_color", &a, &b));
        b.data_mut().base_color = None;
        assert!(!component_equal("minecraft:base_color", &a, &b));
    }

    #[test]
    fn intangible_projectile_equality_borrows_unit_maps() {
        let id = "minecraft:intangible_projectile";
        let mut a = ItemStack::new("minecraft:arrow", 1);
        let mut b = ItemStack::new("minecraft:arrow", 1);
        a.data_mut()
            .set_other(id, Some(Tag::Compound(Default::default())));
        b.data_mut()
            .set_other(id, Some(Tag::compound([("ignored", Tag::Int(42))])));
        assert!(component_equal(id, &a, &b));
        assert!(!stored_component_equal(id, &a, &b));
        b.data_mut().set_other(id, Some(Tag::Byte(1)));
        assert!(!component_equal(id, &a, &b));
        b.data_mut().set_other(id, None);
        assert!(!component_equal(id, &a, &b));
    }
}
