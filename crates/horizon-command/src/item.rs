//! ItemParser (item stacks with components, `stone[damage=5]`) and
//! ItemPredicateArgument (ComponentPredicateParser's packrat grammar:
//! `#logs[count~1..|!damage]`), with vanilla's errors and suggestions.
use super::args::{
    ItemInput, Registries, is_id_char, parse_identifier, read_identifier, suggest_resources,
};
use super::codec::{either, is_number, not_map, snbt};
use super::packrat::{Grammar, Reason, State, Sugg, Term, Value, alt, ch, opt, seq};
use super::{CmdResult, CommandError, Reader, SuggestionsBuilder};
use horizon_core::nbt::{Compound, Tag};
use horizon_items::item_data::component_values;
use horizon_items::properties::swing_animation::SwingAnimation;
use horizon_world::datafix::dynamic::as_number;
use std::rc::Rc;

fn int_of(t: &Tag) -> Option<i64> {
    Some(match t {
        Tag::Byte(v) => i64::from(*v),
        Tag::Short(v) => i64::from(*v),
        Tag::Int(v) => i64::from(*v),
        Tag::Long(v) => *v as i32 as i64,
        Tag::Float(v) => f32::from_bits(*v) as i32 as i64,
        Tag::Double(v) => f64::from_bits(*v) as i32 as i64,
        _ => return None,
    })
}
/// Built-ins with an implemented shared component payload decoder.
pub const DECODED_COMPONENTS: &[&str] = &[
    "minecraft:cat/sound_variant",
    "minecraft:cat/variant",
    "minecraft:chicken/variant",
    "minecraft:wolf/sound_variant",
    "minecraft:wolf/variant",
    "minecraft:container_loot",
    "minecraft:tooltip_display",
    "minecraft:tropical_fish/pattern_color",
    "minecraft:tropical_fish/base_color",
    "minecraft:cushion/color",
    "minecraft:recipes",
    "minecraft:mob_visibility",
    "minecraft:custom_model_data",
    "minecraft:wolf/collar",
    "minecraft:cat/collar",
    "minecraft:shulker/color",
    "minecraft:sheep/color",
    "minecraft:dye",
    "minecraft:block_state",
    "minecraft:use_cooldown",
    "minecraft:cooking_fuel",
    "minecraft:brewing_fuel",
    "minecraft:compostable",
    "minecraft:fox/variant",
    "minecraft:weapon",
    "minecraft:food",
    "minecraft:damage",
    "minecraft:repair_cost",
    "minecraft:max_damage",
    "minecraft:max_stack_size",
    "minecraft:map_id",
    "minecraft:rarity",
    "minecraft:enchantments",
    "minecraft:stored_enchantments",
    "minecraft:enchantment_glint_override",
    "minecraft:custom_name",
    "minecraft:item_name",
    "minecraft:item_model",
    "minecraft:lodestone_tracker",
    "minecraft:damage_resistant",
    "minecraft:use_effects",
    "minecraft:block_entity_data",
    "minecraft:enchantable",
    "minecraft:ominous_bottle_amplifier",
    "minecraft:potion_duration_scale",
    "minecraft:minimum_attack_charge",
    "minecraft:dyed_color",
    "minecraft:attack_animation",
    "minecraft:attack_range",
    "minecraft:rabbit/variant",
    "minecraft:horse/variant",
    "minecraft:llama/variant",
    "minecraft:mooshroom/variant",
    "minecraft:axolotl/variant",
    "minecraft:salmon/size",
    "minecraft:parrot/variant",
    "minecraft:tropical_fish/pattern",
    "minecraft:villager_food",
    "minecraft:unbreakable",
    "minecraft:base_color",
    "minecraft:note_block_sound",
    "minecraft:tooltip_style",
    "minecraft:waxed",
    "minecraft:intangible_projectile",
    "minecraft:glider",
];

/// Validate and decode a component into the value representation shared with stacks.
/// Registered components without a shared decoder retain their exact stored value.
/// Registration is checked by the caller; this helper only rejects failed decodes.
pub fn decoded_component_value(id: &str, value: &Tag, reg: &Registries) -> Result<Tag, String> {
    if id == "minecraft:tooltip_display" {
        return tooltip_display(value, reg);
    }
    if id == "minecraft:attack_animation" {
        return SwingAnimation::decode(value)
            .map(SwingAnimation::to_tag)
            .ok_or_else(|| {
                "Invalid minecraft:attack_animation: expected a swing animation record".into()
            });
    }
    if !DECODED_COMPONENTS.contains(&id) {
        return Ok(value.clone());
    }
    if id == "minecraft:potion_duration_scale" {
        return potion_duration_scale(value).map(|value| Tag::Float(value.to_bits()));
    }
    if matches!(id, "minecraft:item_model" | "minecraft:tooltip_style") {
        return identifier_value(value);
    }
    if id == "minecraft:food" {
        let food = horizon_items::item_data::food_properties(value).map_err(str::to_owned)?;
        return Ok(Tag::compound([
            ("nutrition", Tag::Int(food.nutrition)),
            ("saturation", Tag::Float(food.saturation.to_bits())),
            ("can_always_eat", Tag::Byte(i8::from(food.can_always_eat))),
        ]));
    }
    if id == "minecraft:weapon" {
        return horizon_items::properties::Weapon::from_tag(value)
            .map(|weapon| weapon.to_tag())
            .map_err(str::to_owned);
    }
    if id == "minecraft:compostable" {
        return horizon_items::item_data::ingredient_components::decoded_compostable(value);
    }
    if id == "minecraft:brewing_fuel" {
        return horizon_items::item_data::brewing_fuel::decode(value);
    }
    if id == "minecraft:cooking_fuel" {
        return horizon_items::item_data::cooking_fuel::decode(value);
    }
    if id == "minecraft:use_cooldown" {
        return use_cooldown(value);
    }
    if id == "minecraft:custom_model_data" {
        return horizon_items::item_data::custom_model_data::decode(value)
            .ok_or_else(|| "Expected custom model data record".into());
    }
    if id == "minecraft:mob_visibility" {
        return horizon_items::item_data::ingredient_components::decode_mob_visibility(value, reg);
    }
    if id == "minecraft:recipes" {
        return recipes(value);
    }
    if id == "minecraft:lodestone_tracker" {
        return lodestone_tracker(value);
    }
    if id == "minecraft:container_loot" {
        return container_loot(value);
    }
    if let Some(registry) = horizon_items::properties::pet_variants::registry(id) {
        return horizon_items::properties::pet_variants::decode(registry, value, reg);
    }
    if id == "minecraft:damage_resistant" {
        return component_values::damage_resistant(value, reg);
    }
    if id == "minecraft:use_effects" {
        let (can_sprint, interact_vibrations, speed) =
            horizon_items::item_data::ingredient_components::use_effects(value)
                .map_err(str::to_owned)?;
        return Ok(Tag::Compound(
            [
                ("can_sprint".into(), Tag::Byte(i8::from(can_sprint))),
                (
                    "interact_vibrations".into(),
                    Tag::Byte(i8::from(interact_vibrations)),
                ),
                ("speed_multiplier".into(), Tag::Float(speed)),
            ]
            .into_iter()
            .collect(),
        ));
    }
    if id == "minecraft:attack_range" {
        return horizon_items::properties::attack_range::normalized(value)
            .ok_or_else(|| "Invalid attack_range".into());
    }
    if id == "minecraft:base_color" {
        return dye_color(value).map(|name| Tag::String(name.into()));
    }
    if id == "minecraft:note_block_sound" {
        return identifier_value(value);
    }
    decode_component(id, value, reg)?;
    if id == "minecraft:block_entity_data" {
        let Tag::Compound(_) = value else {
            return Err(not_map(value));
        };
        if !matches!(value.get("id"), Some(Tag::String(_))) {
            return Err("block entity id is not a string".into());
        }
    }
    Ok(match id {
        "minecraft:unbreakable"
        | "minecraft:waxed"
        | "minecraft:intangible_projectile"
        | "minecraft:glider" => Tag::Compound(Compound::new()),
        "minecraft:damage"
        | "minecraft:repair_cost"
        | "minecraft:max_damage"
        | "minecraft:max_stack_size"
        | "minecraft:map_id"
        | "minecraft:ominous_bottle_amplifier" => {
            Tag::Int(int_of(value).expect("validated numeric component") as i32)
        }
        "minecraft:enchantable" => {
            let value = value.get("value").expect("validated enchantable value");
            Tag::compound([(
                "value",
                Tag::Int(int_of(value).expect("validated numeric enchantable value") as i32),
            )])
        }
        "minecraft:minimum_attack_charge" => Tag::Float(
            as_number(value)
                .expect("validated numeric component")
                .float_value()
                .to_bits(),
        ),
        "minecraft:enchantment_glint_override" => Tag::Byte(i8::from(
            horizon_items::item_data::component_bool(value).expect("validated boolean component"),
        )),
        "minecraft:dyed_color" => {
            Tag::Int(horizon_core::color::rgb_color_from_tag(value).expect("validated RGB color"))
        }
        "minecraft:villager_food" => Tag::compound([(
            "nutrition",
            Tag::Int(
                int_of(value.get("nutrition").expect("validated nutrition"))
                    .expect("validated numeric nutrition") as i32,
            ),
        )]),
        "minecraft:enchantments" | "minecraft:stored_enchantments" => {
            let Tag::Compound(entries) = value else {
                unreachable!("validated enchantment map")
            };
            Tag::Compound(
                entries
                    .iter()
                    .map(|(id, level)| {
                        let id = parse_identifier(&id.to_string_lossy())
                            .expect("validated enchantment id");
                        (
                            id.as_str().into(),
                            Tag::Int(int_of(level).expect("validated level") as i32),
                        )
                    })
                    .collect(),
            )
        }
        _ => value.clone(),
    })
}

/// The codecs of the item components commands commonly set.
pub fn decode_component(id: &str, t: &Tag, reg: &Registries) -> Result<(), String> {
    decode_component_depth(id, t, reg, 0)
}

fn decode_component_depth(id: &str, t: &Tag, reg: &Registries, depth: usize) -> Result<(), String> {
    if depth > 16 {
        return Err("Item component nested too deeply".into());
    }
    if let Some(registry) = horizon_items::properties::pet_variants::registry(id) {
        return horizon_items::properties::pet_variants::decode(registry, t, reg).map(|_| ());
    }
    match id {
        "minecraft:container_loot" => container_loot(t).map(|_| ()),
        "minecraft:lodestone_tracker" => lodestone_tracker(t).map(|_| ()),
        "minecraft:recipes" => recipes(t).map(|_| ()),
        "minecraft:mob_visibility" => {
            horizon_items::item_data::ingredient_components::decode_mob_visibility(t, reg)
                .map(|_| ())
        }
        "minecraft:custom_model_data" => horizon_items::item_data::custom_model_data::valid(t)
            .then_some(())
            .ok_or_else(|| "Expected custom model data record".into()),
        // DyeColor.CODEC -> StringRepresentable.fromEnum: ordinary NbtOps uses
        // exact names, not the ordinal codec selected by compressMaps().
        "minecraft:tropical_fish/base_color"
        | "minecraft:tropical_fish/pattern_color"
        | "minecraft:cushion/color"
        | "minecraft:dye"
        | "minecraft:wolf/collar"
        | "minecraft:cat/collar"
        | "minecraft:shulker/color"
        | "minecraft:sheep/color" => match t {
            Tag::String(name) => {
                if horizon_items::firework::DYE_FIREWORK_COLORS
                    .iter()
                    .any(|(color, _)| name.0.iter().copied().eq(color.encode_utf16()))
                {
                    Ok(())
                } else {
                    Err(format!("Unknown element name:{}", name.to_string_lossy()))
                }
            }
            _ => Err("Not a string".into()),
        },

        "minecraft:block_state" => block_state(t),
        "minecraft:use_cooldown" => use_cooldown(t).map(|_| ()),
        "minecraft:cooking_fuel" => horizon_items::item_data::cooking_fuel::decode(t).map(|_| ()),
        "minecraft:brewing_fuel" => horizon_items::item_data::brewing_fuel::decode(t).map(|_| ()),
        "minecraft:compostable" => {
            horizon_items::item_data::ingredient_components::validate_compostable(t)
        }
        "minecraft:fox/variant" => string_enum(t, &["red", "snow"]),
        "minecraft:weapon" => horizon_items::properties::Weapon::from_tag(t)
            .map(|_| ())
            .map_err(str::to_owned),
        "minecraft:food" => horizon_items::item_data::food_properties(t)
            .map(|_| ())
            .map_err(str::to_owned),
        // MapId.CODEC is Codec.INT, without a non-negative constraint.
        "minecraft:map_id" => int_of(t).map(|_| ()).ok_or_else(|| "Not a number".into()),
        "minecraft:dyed_color" => horizon_core::color::rgb_color_from_tag(t)
            .map(|_| ())
            .ok_or_else(|| "Expected a number or a list of three numeric RGB channels".into()),
        "minecraft:attack_animation" => SwingAnimation::decode(t).map(|_| ()).ok_or_else(|| {
            "Invalid minecraft:attack_animation: expected a swing animation record".into()
        }),
        "minecraft:attack_range" => horizon_items::properties::attack_range::decode(t)
            .map(|_| ())
            .ok_or_else(|| "Invalid attack_range".into()),
        // `Unit.CODEC` uses MapCodec.unitCodec: a map is required, its fields are ignored.
        "minecraft:unbreakable"
        | "minecraft:waxed"
        | "minecraft:intangible_projectile"
        | "minecraft:glider" => match t {
            Tag::Compound(_) => Ok(()),
            _ => Err(not_map(t)),
        },
        "minecraft:damage" | "minecraft:repair_cost" => {
            let v = int_of(t).ok_or("Not a number")?;
            if v < 0 {
                return Err(format!("Value must be non-negative: {v}"));
            }
            Ok(())
        }
        "minecraft:max_damage" => {
            let v = int_of(t).ok_or("Not a number")?;
            if v <= 0 {
                return Err(format!("Value must be positive: {v}"));
            }
            Ok(())
        }
        "minecraft:max_stack_size" => component_values::max_stack_size(t).map(|_| ()),
        // Enchantable.CODEC: a record with one required positive integer.
        "minecraft:enchantable" => {
            let Tag::Compound(_) = t else {
                return Err(not_map(t));
            };
            let value = t
                .get("value")
                .ok_or_else(|| format!("No key value in {}", snbt(t)))?;
            let v = int_of(value).ok_or("Not a number")?;
            if v <= 0 {
                return Err(format!("Value must be positive: {v}"));
            }
            Ok(())
        }
        "minecraft:villager_food" => {
            let Tag::Compound(_) = t else {
                return Err(not_map(t));
            };
            let nutrition = t.get("nutrition").ok_or("Missing nutrition")?;
            let value = int_of(nutrition).ok_or("Not a number")?;
            if value <= 0 {
                return Err(format!("Value must be positive: {value}"));
            }
            Ok(())
        }
        // The amplifier integer codec accepts only 0 through 4, without clamping.
        "minecraft:ominous_bottle_amplifier" => {
            let v = int_of(t).ok_or("Not a number")?;
            if !(0..=4).contains(&v) {
                return Err(format!("Value must be within range [0;4]: {v}"));
            }
            Ok(())
        }
        "minecraft:potion_duration_scale" => potion_duration_scale(t).map(|_| ()),
        // DataComponents.MINIMUM_ATTACK_CHARGE / ExtraCodecs.floatRange:
        // validate Number.floatValue with Float.compareTo, which rejects -0.0 and NaN.
        "minecraft:minimum_attack_charge" => {
            let value = as_number(t).ok_or("Not a number")?.float_value();
            if value.is_sign_negative() || !(0.0..=1.0).contains(&value) {
                return Err(format!(
                    "Value must be within range [0.0;1.0]: {}",
                    horizon_core::java_number::float_to_string(value)
                ));
            }
            Ok(())
        }
        "minecraft:rarity" => string_enum(t, &["common", "uncommon", "rare", "epic"]),
        // Rabbit.Variant.CODEC, not the numeric RabbitType legacy codec.
        "minecraft:rabbit/variant" => string_enum(
            t,
            &[
                "brown",
                "white",
                "black",
                "white_splotched",
                "gold",
                "salt",
                "evil",
            ],
        ),
        // equine.Variant.CODEC uses StringRepresentable names under ordinary NbtOps.
        "minecraft:horse/variant" => string_enum(
            t,
            &[
                "white",
                "creamy",
                "chestnut",
                "brown",
                "black",
                "gray",
                "dark_brown",
            ],
        ),
        // Llama.Variant.CODEC uses StringRepresentable names, not LEGACY_CODEC ids.
        "minecraft:llama/variant" => string_enum(t, &["creamy", "white", "brown", "gray"]),
        // MushroomCow.Variant.CODEC uses exact persistent StringRepresentable names.
        "minecraft:mooshroom/variant" => string_enum(t, &["red", "brown"]),
        // Axolotl.Variant.CODEC uses StringRepresentable names under ordinary NbtOps.
        "minecraft:axolotl/variant" => string_enum(t, &["lucy", "wild", "gold", "cyan", "blue"]),
        // Salmon.Variant.CODEC uses StringRepresentable names under ordinary NbtOps.
        "minecraft:salmon/size" => string_enum(t, &["small", "medium", "large"]),
        // Parrot.Variant.CODEC uses StringRepresentable names under ordinary NbtOps.
        "minecraft:parrot/variant" => {
            string_enum(t, &["red_blue", "blue", "green", "yellow_blue", "gray"])
        }
        // TropicalFish.Pattern.CODEC uses serialized names, not packed network ids.
        "minecraft:tropical_fish/pattern" => string_enum(
            t,
            &[
                "kob",
                "sunstreak",
                "snooper",
                "dasher",
                "brinely",
                "spotty",
                "flopper",
                "stripey",
                "glitter",
                "blockfish",
                "betty",
                "clayfish",
            ],
        ),
        "minecraft:base_color" => dye_color(t).map(|_| ()),
        "minecraft:enchantments" | "minecraft:stored_enchantments" => enchantment_map(t, reg),
        "minecraft:tooltip_display" => tooltip_display(t, reg).map(|_| ()),
        // DataComponents.ENCHANTMENT_GLINT_OVERRIDE: Codec.BOOL through NbtOps.
        "minecraft:enchantment_glint_override" => horizon_items::item_data::component_bool(t)
            .map(|_| ())
            .ok_or_else(|| "Not a number".into()),
        "minecraft:custom_name" | "minecraft:item_name" => super::component::decode(t),
        "minecraft:item_model" | "minecraft:tooltip_style" => identifier_value(t).map(|_| ()),
        "minecraft:damage_resistant" => component_values::damage_resistant(t, reg).map(|_| ()),
        "minecraft:use_effects" => horizon_items::item_data::ingredient_components::use_effects(t)
            .map(|_| ())
            .map_err(str::to_owned),
        "minecraft:note_block_sound" => identifier_value(t).map(|_| ()),
        // TypedEntityData.codec: the type is the compound's id.
        "minecraft:block_entity_data" => match t {
            Tag::Compound(c) => match c.get(&"id".into()) {
                None => Err(format!("Expected 'id' field in {}", snbt(t))),
                Some(Tag::String(id)) => {
                    let id = id.to_string_lossy();
                    let id = if id.contains(':') {
                        id
                    } else {
                        format!("minecraft:{id}")
                    };
                    if horizon_client_core::block_entity_nbt::is_known_type(&id) {
                        Ok(())
                    } else {
                        Err(format!(
                            "Unknown registry key in ResourceKey[minecraft:root / minecraft:block_entity_type]: {id}"
                        ))
                    }
                }
                Some(_) => Err("Not a string".into()),
            },
            _ => Err(not_map(t)),
        },
        // CustomData.CODEC: CompoundTag or flattened SNBT compound string.
        "minecraft:custom_data" | "minecraft:bucket_entity_data" => custom_data_component(t),
        // TypedEntityData.codec(EntityType.CODEC): compound with required entity `id`.
        "minecraft:entity_data" => entity_data_component(t, reg),
        // ItemLore.CODEC: ComponentSerialization.CODEC.sizeLimitedListOf(256).
        "minecraft:lore" => lore_component(t),
        // PotionContents.CODEC: record `{potion?, custom_color?, custom_effects?, custom_name?}`
        // or a bare potion identifier.
        "minecraft:potion_contents" => potion_contents_component(t, reg),
        // Consumable.CODEC / DeathProtection.CODEC / Equippable.CODEC / Tool.CODEC.
        "minecraft:consumable" => consumable_component(t, reg),
        "minecraft:death_protection" => death_protection_component(t, reg),
        "minecraft:equippable" => equippable_component(t, reg),
        "minecraft:tool" => tool_component(t, reg),
        // ItemAttributeModifiers.CODEC: Entry.CODEC.listOf().
        "minecraft:attribute_modifiers" => attribute_modifiers_component(t, reg),
        // Book contents.
        "minecraft:writable_book_content" => writable_book_component(t),
        "minecraft:written_book_content" => written_book_component(t),
        // SuspiciousStewEffects / BannerPatternLayers / PotDecorations / ArmorTrim.
        "minecraft:suspicious_stew_effects" => suspicious_stew_component(t, reg),
        "minecraft:banner_patterns" => banner_patterns_component(t),
        "minecraft:pot_decorations" => pot_decorations_component(t, reg, depth + 1),
        "minecraft:trim" => trim_component(t, reg),
        // Sound / Jukebox / Instrument / Map color / Interact animation / Repairable / Villager.
        "minecraft:break_sound" => sound_event_value(t),
        "minecraft:jukebox_playable" => registry_id(t, "minecraft:jukebox_song", reg),
        "minecraft:instrument" => instrument_component(t, reg),
        "minecraft:map_color" => int_of(t).map(|_| ()).ok_or_else(|| "Not a number".into()),
        "minecraft:interact_animation" => SwingAnimation::decode(t).map(|_| ()).ok_or_else(|| {
            "Invalid minecraft:interact_animation: expected a swing animation record".into()
        }),
        "minecraft:repairable" => repairable_component(t, reg),
        "minecraft:use_remainder" => item_template_component(t, reg, depth + 1),
        "minecraft:container" => container_component(t, reg, depth + 1),
        "minecraft:bundle_contents" | "minecraft:charged_projectiles" => {
            item_template_list_component(t, reg, depth + 1)
        }
        "minecraft:bees" => bees_component(t, reg),
        "minecraft:villager/variant" => registry_id(t, "minecraft:villager_type", reg),
        // DataComponents.FIREWORKS / FIREWORK_EXPLOSION.
        "minecraft:fireworks" => fireworks_component(t),
        "minecraft:firework_explosion" => firework_explosion_component(t),
        // ItemPredicateArgument.PSEUDO_COMPONENTS: count uses MinMaxBounds.Ints.CODEC.
        "minecraft:count" => int_bounds(t),
        _ => Ok(()),
    }
}

/// `ExtraCodecs.NON_NEGATIVE_FLOAT`: boxed `Number.floatValue`, then `Float.compareTo`.
/// The inclusive bounds reject negative zero, NaN, and both infinities.
fn potion_duration_scale(t: &Tag) -> Result<f32, String> {
    let value = match t {
        Tag::Byte(value) => f32::from(*value),
        Tag::Short(value) => f32::from(*value),
        Tag::Int(value) => *value as f32,
        Tag::Long(value) => *value as f32,
        Tag::Float(value) => f32::from_bits(*value),
        Tag::Double(value) => f64::from_bits(*value) as f32,
        _ => return Err("Not a number".into()),
    };
    if value.is_finite() && !value.is_sign_negative() {
        Ok(value)
    } else {
        Err(format!("Value must be non-negative: {value}"))
    }
}
/// `DataComponents.ITEM_MODEL`: `Identifier.CODEC`, without a resource lookup.
fn identifier_value(value: &Tag) -> Result<Tag, String> {
    let Tag::String(value) = value else {
        return Err("Not a string".into());
    };
    let text =
        String::from_utf16(&value.0).map_err(|_| "Not a valid resource location".to_owned())?;
    let id = horizon_core::identifier::Identifier::parse(&text)
        .map_err(|error| format!("Not a valid resource location: {text} {error}"))?;
    Ok(Tag::String(id.to_string().as_str().into()))
}
/// `DyeColor.CODEC`: uncompressed persistent values use exact serialized names.
fn dye_color(t: &Tag) -> Result<&'static str, String> {
    let Tag::String(value) = t else {
        return Err("Not a string".into());
    };
    horizon_items::firework::DYE_FIREWORK_COLORS
        .iter()
        .find(|(name, _)| value.0.iter().copied().eq(name.encode_utf16()))
        .map(|(name, _)| *name)
        .ok_or_else(|| format!("Unknown element name:{}", value.to_string_lossy()))
}
/// `ItemEnchantments.CODEC`: registry keys and boxed-number levels in 1..=255.
/// `UnboundedMapCodec` reports failed entries; commands do not retain its partial map.
fn string_enum(t: &Tag, names: &[&str]) -> Result<(), String> {
    let Tag::String(value) = t else {
        return Err("Not a string".into());
    };
    if names
        .iter()
        .any(|name| value.0.iter().copied().eq(name.encode_utf16()))
    {
        Ok(())
    } else {
        Err(format!("Unknown element name:{}", value.to_string_lossy()))
    }
}
fn use_cooldown(value: &Tag) -> Result<Tag, String> {
    let Tag::Compound(_) = value else {
        return Err(not_map(value));
    };
    let (seconds, group) = horizon_items::cooldowns::UseCooldown::fields_from_tag(value)
        .ok_or("Expected numeric 'seconds' field")?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(format!("Value must be positive: {seconds}"));
    }
    let mut record = Compound::new();
    record.insert("seconds".into(), Tag::Float(seconds.to_bits()));
    if let Some(group) = group {
        record.insert("cooldown_group".into(), identifier_value(group)?);
    }
    Ok(Tag::Compound(record))
}
fn block_state(value: &Tag) -> Result<(), String> {
    let Tag::Compound(properties) = value else {
        return Err(not_map(value));
    };
    if properties
        .values()
        .any(|value| !matches!(value, Tag::String(_)))
    {
        return Err("Not a string".into());
    }
    Ok(())
}
fn recipes(value: &Tag) -> Result<Tag, String> {
    let values = match value {
        Tag::List(values) => values.as_slice(),
        // NbtOps.getList accepts every CollectionTag, including empty numeric arrays.
        Tag::ByteArray(values) if values.is_empty() => &[],
        Tag::IntArray(values) if values.is_empty() => &[],
        Tag::LongArray(values) if values.is_empty() => &[],
        Tag::ByteArray(_) | Tag::IntArray(_) | Tag::LongArray(_) => {
            return Err("Not a string".into());
        }
        _ => return Err("Not a list".into()),
    };
    if values.len() > i32::MAX as usize {
        return Err("Recipe list exceeds the codec size limit".into());
    }
    values
        .iter()
        .map(identifier_value)
        .collect::<Result<Vec<_>, _>>()
        .map(Tag::List)
}
fn enchantment_map(t: &Tag, reg: &Registries) -> Result<(), String> {
    let Tag::Compound(map) = t else {
        return Err(not_map(t));
    };
    let mut errors = Vec::new();
    let mut missed = Compound::new();
    for (key, value) in map {
        let mut entry_errors = Vec::new();
        match int_of(value) {
            None => entry_errors.push("Not a number".to_owned()),
            Some(level) if !(1..=255).contains(&level) => {
                entry_errors.push(format!("Value {level} outside of range [1:255]"));
            }
            _ => {}
        }
        let raw = key.to_string_lossy();
        match parse_identifier(&raw) {
            Some(id) if reg.ids("minecraft:enchantment").contains(&id) => {}
            Some(id) => entry_errors.push(format!(
                "Failed to get element {id} from registry minecraft:enchantment"
            )),
            None => entry_errors.push(format!("Not a valid resource location: {raw}")),
        }
        if !entry_errors.is_empty() {
            errors.extend(entry_errors);
            missed.insert(key.clone(), value.clone());
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} missed input: {}",
            errors.join("; "),
            snbt(&Tag::Compound(missed))
        ))
    }
}

/// `MinMaxBounds.Ints.CODEC`: exact int or `{min?, max?}` with `min <= max`.
fn int_bounds(t: &Tag) -> Result<(), String> {
    match t {
        Tag::Compound(c) => {
            let min = match c.get(&"min".into()) {
                Some(v) => Some(int_of(v).ok_or("Not a number")?),
                None => None,
            };
            let max = match c.get(&"max".into()) {
                Some(v) => Some(int_of(v).ok_or("Not a number")?),
                None => None,
            };
            if let (Some(min), Some(max)) = (min, max)
                && min > max
            {
                return Err(format!("Bounds cannot be swapped: min {min} > max {max}"));
            }
            Ok(())
        }
        _ if is_number(t) => Ok(()),
        _ => Err(either(not_map(t), "Not a number".into())),
    }
}

/// `MinMaxBounds.Doubles.CODEC`: exact double or `{min?, max?}` with `min <= max`.
fn double_bounds(t: &Tag) -> Result<(), String> {
    let double_of = |v: &Tag| -> Option<f64> {
        Some(match v {
            Tag::Byte(n) => f64::from(*n),
            Tag::Short(n) => f64::from(*n),
            Tag::Int(n) => f64::from(*n),
            Tag::Long(n) => *n as f64,
            Tag::Float(bits) => f64::from(f32::from_bits(*bits)),
            Tag::Double(bits) => f64::from_bits(*bits),
            _ => return None,
        })
    };
    match t {
        Tag::Compound(c) => {
            let min = match c.get(&"min".into()) {
                Some(v) => Some(double_of(v).ok_or("Not a number")?),
                None => None,
            };
            let max = match c.get(&"max".into()) {
                Some(v) => Some(double_of(v).ok_or("Not a number")?),
                None => None,
            };
            if let (Some(min), Some(max)) = (min, max)
                && min > max
            {
                return Err(format!("Bounds cannot be swapped: min {min} > max {max}"));
            }
            Ok(())
        }
        _ if double_of(t).is_some() => Ok(()),
        _ => Err(either(not_map(t), "Not a number".into())),
    }
}

fn bool_value(t: &Tag) -> Result<(), String> {
    horizon_items::item_data::component_bool(t)
        .map(|_| ())
        .ok_or_else(|| "Not a number".into())
}

fn string_value(t: &Tag) -> Result<(), String> {
    if matches!(t, Tag::String(_)) {
        Ok(())
    } else {
        Err("Not a string".into())
    }
}

fn list_elements<'a>(t: &'a Tag, elem_name: &str) -> Result<&'a [Tag], String> {
    match t {
        Tag::List(items) => Ok(items.as_slice()),
        Tag::ByteArray(items) if items.is_empty() => Ok(&[]),
        Tag::IntArray(items) if items.is_empty() => Ok(&[]),
        Tag::LongArray(items) if items.is_empty() => Ok(&[]),
        Tag::ByteArray(_) | Tag::IntArray(_) | Tag::LongArray(_) => Err(elem_name.into()),
        _ => Err("Not a list".into()),
    }
}

/// `HolderSetCodec`: `"#tag"`, `"id"`, or `["id", ...]`.
fn holder_set(t: &Tag, registry: &str, reg: &Registries) -> Result<(), String> {
    let check_id = |raw: &str| -> Result<(), String> {
        let id =
            parse_identifier(raw).ok_or_else(|| format!("Not a valid resource location: {raw}"))?;
        let known = reg.ids(registry);
        if !known.is_empty() && !known.iter().any(|k| k == &id) {
            return Err(format!(
                "Failed to get element {id} from registry {registry}"
            ));
        }
        Ok(())
    };
    match t {
        Tag::String(value) => {
            let text = String::from_utf16(&value.0)
                .map_err(|_| "Not a valid resource location".to_owned())?;
            if let Some(tag_raw) = text.strip_prefix('#') {
                let tag = parse_identifier(tag_raw)
                    .ok_or_else(|| format!("Not a valid resource location: {tag_raw}"))?;
                let tags = reg.tags(registry);
                if !tags.is_empty() && !tags.iter().any(|k| k == &tag) {
                    return Err(format!("Unknown tag #{tag} in registry {registry}"));
                }
                Ok(())
            } else {
                check_id(&text)
            }
        }
        Tag::List(items) => {
            for item in items {
                let Tag::String(value) = item else {
                    return Err("Not a string".into());
                };
                let text = String::from_utf16(&value.0)
                    .map_err(|_| "Not a valid resource location".to_owned())?;
                check_id(&text)?;
            }
            Ok(())
        }
        Tag::ByteArray(v) if v.is_empty() => Ok(()),
        Tag::IntArray(v) if v.is_empty() => Ok(()),
        Tag::LongArray(v) if v.is_empty() => Ok(()),
        _ => Err(format!("Not a string or list: {}", snbt(t))),
    }
}

/// `CollectionPredicate.codec(elementCodec)`: `{contains?, count?, size?}`.
fn collection_predicate(
    t: &Tag,
    elem_check: &dyn Fn(&Tag) -> Result<(), String>,
) -> Result<(), String> {
    let Tag::Compound(c) = t else {
        return Err(not_map(t));
    };
    if let Some(contains) = c.get(&"contains".into()) {
        for elem in list_elements(contains, "Invalid collection predicate entry")? {
            elem_check(elem)?;
        }
    }
    if let Some(counts) = c.get(&"count".into()) {
        for entry in list_elements(counts, "Not a map")? {
            let Tag::Compound(ec) = entry else {
                return Err(not_map(entry));
            };
            let test = ec
                .get(&"test".into())
                .ok_or_else(|| format!("No key test in {}", snbt(entry)))?;
            elem_check(test)?;
            let count = ec
                .get(&"count".into())
                .ok_or_else(|| format!("No key count in {}", snbt(entry)))?;
            int_bounds(count)?;
        }
    }
    if let Some(size) = c.get(&"size".into()) {
        int_bounds(size)?;
    }
    Ok(())
}

fn firework_explosion_predicate(t: &Tag) -> Result<(), String> {
    let Tag::Compound(c) = t else {
        return Err(not_map(t));
    };
    if let Some(shape) = c.get(&"shape".into()) {
        string_enum(
            shape,
            &["small_ball", "large_ball", "star", "creeper", "burst"],
        )?;
    }
    if let Some(twinkle) = c.get(&"has_twinkle".into()) {
        bool_value(twinkle)?;
    }
    if let Some(trail) = c.get(&"has_trail".into()) {
        bool_value(trail)?;
    }
    Ok(())
}

fn firework_explosion_component(t: &Tag) -> Result<(), String> {
    let Tag::Compound(c) = t else {
        return Err(not_map(t));
    };
    let shape = c
        .get(&"shape".into())
        .ok_or_else(|| format!("No key shape in {}", snbt(t)))?;
    string_enum(
        shape,
        &["small_ball", "large_ball", "star", "creeper", "burst"],
    )?;
    for key in ["colors", "fade_colors"] {
        if let Some(list) = c.get(&key.into()) {
            match list {
                Tag::IntArray(_) | Tag::ByteArray(_) | Tag::LongArray(_) => {}
                Tag::List(items) => {
                    for item in items {
                        if int_of(item).is_none() {
                            return Err("Not a number".into());
                        }
                    }
                }
                _ => return Err("Not a list".into()),
            }
        }
    }
    if let Some(trail) = c.get(&"has_trail".into()) {
        bool_value(trail)?;
    }
    if let Some(twinkle) = c.get(&"has_twinkle".into()) {
        bool_value(twinkle)?;
    }
    Ok(())
}

fn fireworks_component(t: &Tag) -> Result<(), String> {
    let Tag::Compound(c) = t else {
        return Err(not_map(t));
    };
    if let Some(flight) = c.get(&"flight_duration".into()) {
        let v = int_of(flight).ok_or("Not a number")?;
        if !(0..=255).contains(&v) {
            return Err(format!("Value must be unsigned byte: {v}"));
        }
    }
    if let Some(explosions) = c.get(&"explosions".into()) {
        let items = list_elements(explosions, "Not a map")?;
        if items.len() > 256 {
            return Err(format!(
                "List is too long: {}, expected range [0-256]",
                items.len()
            ));
        }
        for item in items {
            firework_explosion_component(item)?;
        }
    }
    Ok(())
}

fn registry_id(t: &Tag, registry: &str, reg: &Registries) -> Result<(), String> {
    let normalized = identifier_value(t)?;
    let Tag::String(id) = normalized else {
        unreachable!("identifier_value returns string")
    };
    let id = id.to_string_lossy();
    let known = reg.ids(registry);
    if !known.is_empty() && !known.iter().any(|k| k == &id) {
        return Err(format!(
            "Failed to get element {id} from registry {registry}"
        ));
    }
    Ok(())
}

fn custom_data_component(t: &Tag) -> Result<(), String> {
    match t {
        Tag::Compound(_) => Ok(()),
        Tag::String(s) => match super::snbt::parse_fully(&s.to_string_lossy()) {
            Ok(Tag::Compound(_)) => Ok(()),
            Ok(other) => Err(not_map(&other)),
            Err(e) => Err(e.message.plain()),
        },
        _ => Err(either(not_map(t), "Not a string".into())),
    }
}

fn entity_data_component(t: &Tag, reg: &Registries) -> Result<(), String> {
    let Tag::Compound(c) = t else {
        return Err(not_map(t));
    };
    let id = c
        .get(&"id".into())
        .ok_or_else(|| format!("Expected 'id' field in {}", snbt(t)))?;
    registry_id(id, "minecraft:entity_type", reg)
}

fn lore_component(t: &Tag) -> Result<(), String> {
    let items = list_elements(t, "Invalid text component")?;
    if items.len() > 256 {
        return Err(format!(
            "List is too long: {}, expected range [0-256]",
            items.len()
        ));
    }
    for item in items {
        super::component::decode(item)?;
    }
    Ok(())
}

fn sound_event_value(t: &Tag) -> Result<(), String> {
    match t {
        Tag::String(_) => identifier_value(t).map(|_| ()),
        Tag::Compound(c) => {
            let sound_id = c
                .get(&"sound_id".into())
                .ok_or_else(|| format!("No key sound_id in {}", snbt(t)))?;
            identifier_value(sound_id)?;
            if let Some(range) = c.get(&"range".into()) {
                let v = as_number(range).ok_or("Not a number")?.float_value();
                if !v.is_finite() {
                    return Err(format!("Invalid sound range: {v}"));
                }
            }
            Ok(())
        }
        _ => Err(either("Not a string".into(), not_map(t))),
    }
}

/// `MobEffectInstance.Details.MAP_CODEC` (recursive `hidden_effect` has no `id`).
fn mob_effect_details_component(c: &Compound, depth: usize) -> Result<(), String> {
    if depth > 16 {
        return Err("Mob effect nested too deeply".into());
    }
    if let Some(amp) = c.get(&"amplifier".into()) {
        let v = int_of(amp).ok_or("Not a number")?;
        if !(0..=255).contains(&v) {
            return Err(format!("Value must be unsigned byte: {v}"));
        }
    }
    if let Some(dur) = c.get(&"duration".into())
        && int_of(dur).is_none()
    {
        return Err("Not a number".into());
    }
    for key in ["ambient", "show_particles", "show_icon"] {
        if let Some(v) = c.get(&key.into()) {
            bool_value(v)?;
        }
    }
    if let Some(hidden) = c.get(&"hidden_effect".into()) {
        let Tag::Compound(hc) = hidden else {
            return Err(not_map(hidden));
        };
        mob_effect_details_component(hc, depth + 1)?;
    }
    Ok(())
}

/// `MobEffectInstance.CODEC`: required `id` plus `Details` fields.
fn mob_effect_instance_component(t: &Tag, reg: &Registries) -> Result<(), String> {
    let Tag::Compound(c) = t else {
        return Err(not_map(t));
    };
    let id = c
        .get(&"id".into())
        .ok_or_else(|| format!("No key id in {}", snbt(t)))?;
    registry_id(id, "minecraft:mob_effect", reg)?;
    mob_effect_details_component(c, 0)
}

fn potion_contents_component(t: &Tag, reg: &Registries) -> Result<(), String> {
    match t {
        Tag::String(_) => registry_id(t, "minecraft:potion", reg),
        Tag::Compound(c) => {
            if let Some(potion) = c.get(&"potion".into()) {
                registry_id(potion, "minecraft:potion", reg)?;
            }
            if let Some(color) = c.get(&"custom_color".into())
                && int_of(color).is_none()
            {
                return Err("Not a number".into());
            }
            if let Some(effects) = c.get(&"custom_effects".into()) {
                for effect in list_elements(effects, "Not a map")? {
                    mob_effect_instance_component(effect, reg)?;
                }
            }
            if let Some(name) = c.get(&"custom_name".into()) {
                string_value(name)?;
            }
            Ok(())
        }
        _ => Err(either(not_map(t), "Not a string".into())),
    }
}

fn consume_effect_component(t: &Tag, reg: &Registries) -> Result<(), String> {
    let Tag::Compound(c) = t else {
        return Err(not_map(t));
    };
    let ty_tag = c
        .get(&"type".into())
        .ok_or_else(|| format!("No key type in {}", snbt(t)))?;
    let Tag::String(ty_norm) = identifier_value(ty_tag)? else {
        unreachable!("identifier_value returns string")
    };
    match ty_norm.to_string_lossy().as_str() {
        "minecraft:apply_effects" => {
            let effects = c
                .get(&"effects".into())
                .ok_or_else(|| format!("No key effects in {}", snbt(t)))?;
            for effect in list_elements(effects, "Not a map")? {
                mob_effect_instance_component(effect, reg)?;
            }
            if let Some(prob) = c.get(&"probability".into()) {
                let v = as_number(prob).ok_or("Not a number")?.float_value();
                if v.is_sign_negative() || !(0.0..=1.0).contains(&v) {
                    return Err(format!("Value must be within range [0.0;1.0]: {v}"));
                }
            }
            Ok(())
        }
        "minecraft:remove_effects" => {
            let effects = c
                .get(&"effects".into())
                .ok_or_else(|| format!("No key effects in {}", snbt(t)))?;
            holder_set(effects, "minecraft:mob_effect", reg)
        }
        "minecraft:clear_all_effects" => Ok(()),
        "minecraft:teleport_randomly" => {
            if let Some(diam) = c.get(&"diameter".into()) {
                let v = as_number(diam).ok_or("Not a number")?.float_value();
                if !v.is_finite() || v <= 0.0 {
                    return Err(format!("Value must be positive: {v}"));
                }
            }
            Ok(())
        }
        "minecraft:play_sound" => {
            let sound = c
                .get(&"sound".into())
                .ok_or_else(|| format!("No key sound in {}", snbt(t)))?;
            sound_event_value(sound)
        }
        other => Err(format!("Unknown consume effect type: {other}")),
    }
}

fn consumable_component(t: &Tag, reg: &Registries) -> Result<(), String> {
    let Tag::Compound(c) = t else {
        return Err(not_map(t));
    };
    if let Some(seconds) = c.get(&"consume_seconds".into()) {
        potion_duration_scale(seconds)?;
    }
    if let Some(anim) = c.get(&"animation".into()) {
        string_enum(
            anim,
            &[
                "none",
                "eat",
                "drink",
                "block",
                "bow",
                "trident",
                "crossbow",
                "spyglass",
                "toot_horn",
                "brush",
                "bundle",
                "spear",
            ],
        )?;
    }
    if let Some(sound) = c.get(&"sound".into()) {
        sound_event_value(sound)?;
    }
    if let Some(particles) = c.get(&"has_consume_particles".into()) {
        bool_value(particles)?;
    }
    if let Some(effects) = c.get(&"on_consume_effects".into()) {
        for effect in list_elements(effects, "Not a map")? {
            consume_effect_component(effect, reg)?;
        }
    }
    Ok(())
}

fn death_protection_component(t: &Tag, reg: &Registries) -> Result<(), String> {
    let Tag::Compound(c) = t else {
        return Err(not_map(t));
    };
    if let Some(effects) = c.get(&"death_effects".into()) {
        for effect in list_elements(effects, "Not a map")? {
            consume_effect_component(effect, reg)?;
        }
    }
    Ok(())
}

fn equippable_component(t: &Tag, reg: &Registries) -> Result<(), String> {
    let Tag::Compound(c) = t else {
        return Err(not_map(t));
    };
    let slot = c
        .get(&"slot".into())
        .ok_or_else(|| format!("No key slot in {}", snbt(t)))?;
    string_enum(
        slot,
        &[
            "mainhand", "offhand", "feet", "legs", "chest", "head", "body", "saddle",
        ],
    )?;
    if let Some(sound) = c.get(&"equip_sound".into()) {
        sound_event_value(sound)?;
    }
    if let Some(asset) = c.get(&"asset_id".into()) {
        identifier_value(asset)?;
    }
    if let Some(overlay) = c.get(&"camera_overlay".into()) {
        identifier_value(overlay)?;
    }
    if let Some(entities) = c.get(&"allowed_entities".into()) {
        holder_set(entities, "minecraft:entity_type", reg)?;
    }
    for key in [
        "dispensable",
        "swappable",
        "damage_on_hurt",
        "equip_on_interact",
        "can_be_sheared",
    ] {
        if let Some(v) = c.get(&key.into()) {
            bool_value(v)?;
        }
    }
    if let Some(sound) = c.get(&"shearing_sound".into()) {
        sound_event_value(sound)?;
    }
    Ok(())
}

fn tool_component(t: &Tag, reg: &Registries) -> Result<(), String> {
    let Tag::Compound(c) = t else {
        return Err(not_map(t));
    };
    let rules = c
        .get(&"rules".into())
        .ok_or_else(|| format!("No key rules in {}", snbt(t)))?;
    for rule in list_elements(rules, "Not a map")? {
        let Tag::Compound(rc) = rule else {
            return Err(not_map(rule));
        };
        let blocks = rc
            .get(&"blocks".into())
            .ok_or_else(|| format!("No key blocks in {}", snbt(rule)))?;
        holder_set(blocks, "minecraft:block", reg)?;
        if let Some(speed) = rc.get(&"speed".into()) {
            let v = as_number(speed).ok_or("Not a number")?.float_value();
            if !v.is_finite() || v <= 0.0 {
                return Err(format!("Value must be positive: {v}"));
            }
        }
        if let Some(drops) = rc.get(&"correct_for_drops".into()) {
            bool_value(drops)?;
        }
    }
    if let Some(speed) = c.get(&"default_mining_speed".into()) {
        let v = as_number(speed).ok_or("Not a number")?.float_value();
        if !v.is_finite() {
            return Err(format!("Invalid mining speed: {v}"));
        }
    }
    if let Some(dpb) = c.get(&"damage_per_block".into()) {
        let v = int_of(dpb).ok_or("Not a number")?;
        if v < 0 {
            return Err(format!("Value must be non-negative: {v}"));
        }
    }
    if let Some(creative) = c.get(&"can_destroy_blocks_in_creative".into()) {
        bool_value(creative)?;
    }
    Ok(())
}

fn attribute_modifiers_component(t: &Tag, reg: &Registries) -> Result<(), String> {
    for entry in list_elements(t, "Not a map")? {
        let Tag::Compound(c) = entry else {
            return Err(not_map(entry));
        };
        let attr = c
            .get(&"type".into())
            .ok_or_else(|| format!("No key type in {}", snbt(entry)))?;
        registry_id(attr, "minecraft:attribute", reg)?;
        let id = c
            .get(&"id".into())
            .ok_or_else(|| format!("No key id in {}", snbt(entry)))?;
        identifier_value(id)?;
        let amount = c
            .get(&"amount".into())
            .ok_or_else(|| format!("No key amount in {}", snbt(entry)))?;
        if as_number(amount).is_none() {
            return Err("Not a number".into());
        }
        let op = c
            .get(&"operation".into())
            .ok_or_else(|| format!("No key operation in {}", snbt(entry)))?;
        string_enum(
            op,
            &["add_value", "add_multiplied_base", "add_multiplied_total"],
        )?;
        if let Some(slot) = c.get(&"slot".into()) {
            string_enum(
                slot,
                &[
                    "any", "mainhand", "offhand", "hand", "feet", "legs", "chest", "head", "armor",
                    "body", "saddle",
                ],
            )?;
        }
        if let Some(display) = c.get(&"display".into()) {
            let Tag::Compound(dc) = display else {
                return Err(not_map(display));
            };
            let dty = dc
                .get(&"type".into())
                .ok_or_else(|| format!("No key type in {}", snbt(display)))?;
            string_enum(dty, &["default", "hidden", "override"])?;
            if let Tag::String(s) = dty
                && s.to_string_lossy() == "override"
            {
                let value = dc
                    .get(&"value".into())
                    .ok_or_else(|| format!("No key value in {}", snbt(display)))?;
                super::component::decode(value)?;
            }
        }
    }
    Ok(())
}

fn filterable_string(t: &Tag, max_len: usize) -> Result<(), String> {
    let check_str = |v: &Tag| -> Result<(), String> {
        let Tag::String(s) = v else {
            return Err("Not a string".into());
        };
        if s.0.len() > max_len {
            return Err(format!("String too long: {} > {max_len}", s.0.len()));
        }
        Ok(())
    };
    match t {
        Tag::String(_) => check_str(t),
        Tag::Compound(c) => {
            let raw = c
                .get(&"raw".into())
                .ok_or_else(|| format!("No key raw in {}", snbt(t)))?;
            check_str(raw)?;
            if let Some(filtered) = c.get(&"filtered".into()) {
                check_str(filtered)?;
            }
            Ok(())
        }
        _ => Err(either(not_map(t), "Not a string".into())),
    }
}

fn writable_book_component(t: &Tag) -> Result<(), String> {
    let Tag::Compound(c) = t else {
        return Err(not_map(t));
    };
    if let Some(pages) = c.get(&"pages".into()) {
        let items = list_elements(pages, "Not a string")?;
        if items.len() > 100 {
            return Err(format!(
                "List is too long: {}, expected range [0-100]",
                items.len()
            ));
        }
        for page in items {
            filterable_string(page, 1024)?;
        }
    }
    Ok(())
}

fn written_book_component(t: &Tag) -> Result<(), String> {
    let Tag::Compound(c) = t else {
        return Err(not_map(t));
    };
    let title = c
        .get(&"title".into())
        .ok_or_else(|| format!("No key title in {}", snbt(t)))?;
    filterable_string(title, 32)?;
    let author = c
        .get(&"author".into())
        .ok_or_else(|| format!("No key author in {}", snbt(t)))?;
    string_value(author)?;
    if let Some(generation) = c.get(&"generation".into()) {
        let v = int_of(generation).ok_or("Not a number")?;
        if !(0..=3).contains(&v) {
            return Err(format!("Value must be within range [0;3]: {v}"));
        }
    }
    if let Some(pages) = c.get(&"pages".into()) {
        for page in list_elements(pages, "Invalid text component")? {
            if let Tag::Compound(pc) = page
                && let Some(raw) = pc.get(&"raw".into())
            {
                super::component::decode(raw)?;
                if let Some(filtered) = pc.get(&"filtered".into()) {
                    super::component::decode(filtered)?;
                }
            } else {
                super::component::decode(page)?;
            }
        }
    }
    if let Some(resolved) = c.get(&"resolved".into()) {
        bool_value(resolved)?;
    }
    Ok(())
}

fn suspicious_stew_component(t: &Tag, reg: &Registries) -> Result<(), String> {
    for entry in list_elements(t, "Not a map")? {
        let Tag::Compound(c) = entry else {
            return Err(not_map(entry));
        };
        let id = c
            .get(&"id".into())
            .ok_or_else(|| format!("No key id in {}", snbt(entry)))?;
        registry_id(id, "minecraft:mob_effect", reg)?;
        if let Some(dur) = c.get(&"duration".into())
            && int_of(dur).is_none()
        {
            return Err("Not a number".into());
        }
    }
    Ok(())
}

fn banner_patterns_component(t: &Tag) -> Result<(), String> {
    for entry in list_elements(t, "Not a map")? {
        let Tag::Compound(c) = entry else {
            return Err(not_map(entry));
        };
        let pattern = c
            .get(&"pattern".into())
            .ok_or_else(|| format!("No key pattern in {}", snbt(entry)))?;
        match pattern {
            Tag::String(_) => identifier_value(pattern).map(|_| ())?,
            Tag::Compound(pc) => {
                let asset = pc
                    .get(&"asset_id".into())
                    .ok_or_else(|| format!("No key asset_id in {}", snbt(pattern)))?;
                identifier_value(asset)?;
                let key = pc
                    .get(&"translation_key".into())
                    .ok_or_else(|| format!("No key translation_key in {}", snbt(pattern)))?;
                string_value(key)?;
            }
            _ => return Err(either("Not a string".into(), not_map(pattern))),
        }
        let color = c
            .get(&"color".into())
            .ok_or_else(|| format!("No key color in {}", snbt(entry)))?;
        dye_color(color)?;
    }
    Ok(())
}

fn pot_decorations_component(t: &Tag, reg: &Registries, depth: usize) -> Result<(), String> {
    let Tag::Compound(c) = t else {
        return Err(not_map(t));
    };
    for side in ["back", "left", "right", "front"] {
        if let Some(item) = c.get(&side.into()) {
            item_template_component(item, reg, depth)?;
        }
    }
    Ok(())
}

fn trim_material_holder(t: &Tag, reg: &Registries) -> Result<(), String> {
    match t {
        Tag::String(_) => registry_id(t, "minecraft:trim_material", reg),
        Tag::Compound(c) => {
            let palette = c
                .get(&"palette_id".into())
                .ok_or_else(|| format!("No key palette_id in {}", snbt(t)))?;
            identifier_value(palette)?;
            let desc = c
                .get(&"description".into())
                .ok_or_else(|| format!("No key description in {}", snbt(t)))?;
            super::component::decode(desc)
        }
        _ => Err(either("Not a string".into(), not_map(t))),
    }
}

fn trim_pattern_holder(t: &Tag, reg: &Registries) -> Result<(), String> {
    match t {
        Tag::String(_) => registry_id(t, "minecraft:trim_pattern", reg),
        Tag::Compound(c) => {
            let asset = c
                .get(&"asset_id".into())
                .ok_or_else(|| format!("No key asset_id in {}", snbt(t)))?;
            identifier_value(asset)?;
            let desc = c
                .get(&"description".into())
                .ok_or_else(|| format!("No key description in {}", snbt(t)))?;
            super::component::decode(desc)?;
            if let Some(decal) = c.get(&"decal".into()) {
                bool_value(decal)?;
            }
            Ok(())
        }
        _ => Err(either("Not a string".into(), not_map(t))),
    }
}

fn trim_component(t: &Tag, reg: &Registries) -> Result<(), String> {
    let Tag::Compound(c) = t else {
        return Err(not_map(t));
    };
    let material = c
        .get(&"material".into())
        .ok_or_else(|| format!("No key material in {}", snbt(t)))?;
    trim_material_holder(material, reg)?;
    let pattern = c
        .get(&"pattern".into())
        .ok_or_else(|| format!("No key pattern in {}", snbt(t)))?;
    trim_pattern_holder(pattern, reg)
}

fn instrument_component(t: &Tag, reg: &Registries) -> Result<(), String> {
    match t {
        Tag::String(_) => registry_id(t, "minecraft:instrument", reg),
        Tag::Compound(c) => {
            let sound = c
                .get(&"sound_event".into())
                .ok_or_else(|| format!("No key sound_event in {}", snbt(t)))?;
            sound_event_value(sound)?;
            let use_dur = c
                .get(&"use_duration".into())
                .ok_or_else(|| format!("No key use_duration in {}", snbt(t)))?;
            let dur = as_number(use_dur).ok_or("Not a number")?.float_value();
            if !dur.is_finite() || dur <= 0.0 {
                return Err(format!("Value must be positive: {dur}"));
            }
            let range = c
                .get(&"range".into())
                .ok_or_else(|| format!("No key range in {}", snbt(t)))?;
            let r = as_number(range).ok_or("Not a number")?.float_value();
            if !r.is_finite() || r <= 0.0 {
                return Err(format!("Value must be positive: {r}"));
            }
            let desc = c
                .get(&"description".into())
                .ok_or_else(|| format!("No key description in {}", snbt(t)))?;
            super::component::decode(desc)
        }
        _ => Err(either("Not a string".into(), not_map(t))),
    }
}

fn repairable_component(t: &Tag, reg: &Registries) -> Result<(), String> {
    let Tag::Compound(c) = t else {
        return Err(not_map(t));
    };
    let items = c
        .get(&"items".into())
        .ok_or_else(|| format!("No key items in {}", snbt(t)))?;
    holder_set(items, "minecraft:item", reg)
}

fn item_template_component(t: &Tag, reg: &Registries, depth: usize) -> Result<(), String> {
    if depth > 16 {
        return Err("Item template nested too deeply".into());
    }
    let Tag::Compound(c) = t else {
        return Err(not_map(t));
    };
    let id = c
        .get(&"id".into())
        .ok_or_else(|| format!("No key id in {}", snbt(t)))?;
    let Tag::String(norm_id) = identifier_value(id)? else {
        unreachable!("identifier_value returns string")
    };
    let norm_str = norm_id.to_string_lossy();
    if norm_str == "minecraft:air" {
        return Err("Item must not be minecraft:air".into());
    }
    let known_items = reg.ids("minecraft:item");
    if !known_items.is_empty() && !known_items.iter().any(|k| k == &norm_str) {
        return Err(format!(
            "Failed to get element {norm_str} from registry minecraft:item"
        ));
    }
    if let Some(count) = c.get(&"count".into()) {
        let v = int_of(count).ok_or("Not a number")?;
        if !(1..=99).contains(&v) {
            return Err(format!("Value must be within range [1;99]: {v}"));
        }
    }
    if let Some(comps) = c.get(&"components".into()) {
        let Tag::Compound(patch) = comps else {
            return Err(not_map(comps));
        };
        for (k, v) in patch {
            let raw = k.to_string_lossy();
            if let Some(rem) = raw.strip_prefix('!') {
                let cid = parse_identifier(rem)
                    .ok_or_else(|| format!("Not a valid resource location: {rem}"))?;
                if !reg.persistent_components.is_empty()
                    && !reg.persistent_components.iter().any(|c| c == &cid)
                {
                    return Err(format!("Unknown data component type: {cid}"));
                }
                if !matches!(v, Tag::Compound(_)) {
                    return Err(not_map(v));
                }
            } else {
                let cid = parse_identifier(&raw)
                    .ok_or_else(|| format!("Not a valid resource location: {raw}"))?;
                if !reg.persistent_components.is_empty()
                    && !reg.persistent_components.iter().any(|c| c == &cid)
                {
                    return Err(format!("Unknown data component type: {cid}"));
                }
                decode_component_depth(&cid, v, reg, depth + 1)?;
            }
        }
    }
    Ok(())
}

fn item_template_list_component(t: &Tag, reg: &Registries, depth: usize) -> Result<(), String> {
    for item in list_elements(t, "Not a map")? {
        item_template_component(item, reg, depth)?;
    }
    Ok(())
}

fn container_component(t: &Tag, reg: &Registries, depth: usize) -> Result<(), String> {
    for entry in list_elements(t, "Not a map")? {
        let Tag::Compound(c) = entry else {
            return Err(not_map(entry));
        };
        let slot = c
            .get(&"slot".into())
            .ok_or_else(|| format!("No key slot in {}", snbt(entry)))?;
        let s = int_of(slot).ok_or("Not a number")?;
        if !(0..=255).contains(&s) {
            return Err(format!("Value must be within range [0;255]: {s}"));
        }
        let item = c
            .get(&"item".into())
            .ok_or_else(|| format!("No key item in {}", snbt(entry)))?;
        item_template_component(item, reg, depth)?;
    }
    Ok(())
}

fn bees_component(t: &Tag, reg: &Registries) -> Result<(), String> {
    for entry in list_elements(t, "Not a map")? {
        let Tag::Compound(c) = entry else {
            return Err(not_map(entry));
        };
        let entity = c
            .get(&"entity_data".into())
            .ok_or_else(|| format!("No key entity_data in {}", snbt(entry)))?;
        entity_data_component(entity, reg)?;
        let ticks = c
            .get(&"ticks_in_hive".into())
            .ok_or_else(|| format!("No key ticks_in_hive in {}", snbt(entry)))?;
        if int_of(ticks).is_none() {
            return Err("Not a number".into());
        }
        let min_ticks = c
            .get(&"min_ticks_in_hive".into())
            .ok_or_else(|| format!("No key min_ticks_in_hive in {}", snbt(entry)))?;
        if int_of(min_ticks).is_none() {
            return Err("Not a number".into());
        }
    }
    Ok(())
}

fn mob_effects_predicate(t: &Tag, reg: &Registries) -> Result<(), String> {
    let Tag::Compound(map) = t else {
        return Err(not_map(t));
    };
    let known = reg.ids("minecraft:mob_effect");
    for (k, v) in map {
        let raw = k.to_string_lossy();
        let id = parse_identifier(&raw)
            .ok_or_else(|| format!("Not a valid resource location: {raw}"))?;
        if !known.is_empty() && !known.iter().any(|e| e == &id) {
            return Err(format!(
                "Failed to get element {id} from registry minecraft:mob_effect"
            ));
        }
        let Tag::Compound(entry) = v else {
            return Err(not_map(v));
        };
        if let Some(amp) = entry.get(&"amplifier".into()) {
            int_bounds(amp)?;
        }
        if let Some(dur) = entry.get(&"duration".into()) {
            int_bounds(dur)?;
        }
        if let Some(ambient) = entry.get(&"ambient".into()) {
            bool_value(ambient)?;
        }
        if let Some(visible) = entry.get(&"visible".into()) {
            bool_value(visible)?;
        }
    }
    Ok(())
}

fn attribute_modifier_entry_predicate(t: &Tag, reg: &Registries) -> Result<(), String> {
    let Tag::Compound(c) = t else {
        return Err(not_map(t));
    };
    if let Some(attr) = c.get(&"attribute".into()) {
        holder_set(attr, "minecraft:attribute", reg)?;
    }
    if let Some(id) = c.get(&"id".into()) {
        identifier_value(id)?;
    }
    if let Some(amount) = c.get(&"amount".into()) {
        double_bounds(amount)?;
    }
    if let Some(op) = c.get(&"operation".into()) {
        string_enum(
            op,
            &["add_value", "add_multiplied_base", "add_multiplied_total"],
        )?;
    }
    if let Some(slot) = c.get(&"slot".into()) {
        string_enum(
            slot,
            &[
                "any", "mainhand", "offhand", "hand", "feet", "legs", "chest", "head", "armor",
                "body", "saddle",
            ],
        )?;
    }
    Ok(())
}

fn item_predicate_tag(t: &Tag, reg: &Registries, depth: usize) -> Result<(), String> {
    if depth > 16 {
        return Err("Item predicate nested too deeply".into());
    }
    let Tag::Compound(c) = t else {
        return Err(not_map(t));
    };
    if let Some(items) = c.get(&"items".into()) {
        holder_set(items, "minecraft:item", reg)?;
    }
    if let Some(count) = c.get(&"count".into()) {
        int_bounds(count)?;
    }
    if let Some(components) = c.get(&"components".into()) {
        let Tag::Compound(map) = components else {
            return Err(not_map(components));
        };
        for (k, v) in map {
            let raw = k.to_string_lossy();
            let id = parse_identifier(&raw)
                .ok_or_else(|| format!("Not a valid resource location: {raw}"))?;
            if !reg.persistent_components.is_empty()
                && !reg.persistent_components.iter().any(|c| c == &id)
            {
                return Err(format!("Unknown data component type: {id}"));
            }
            decode_component(&id, v, reg)?;
        }
    }
    if let Some(predicates) = c.get(&"predicates".into()) {
        let Tag::Compound(map) = predicates else {
            return Err(not_map(predicates));
        };
        for (k, v) in map {
            let raw = k.to_string_lossy();
            let id = parse_identifier(&raw)
                .ok_or_else(|| format!("Not a valid resource location: {raw}"))?;
            let known_preds = reg.ids("minecraft:data_component_predicate_type");
            let known_comps = reg.ids("minecraft:data_component_type");
            if !known_preds.is_empty() && !known_preds.iter().any(|p| p == &id) {
                if known_comps.iter().any(|c| c == &id) {
                    decode_predicate_depth(&format!("exists:{id}"), v, reg, depth + 1)?;
                    continue;
                }
                return Err(format!("Unknown data component predicate type: {id}"));
            }
            decode_predicate_depth(&id, v, reg, depth + 1)?;
        }
    }
    Ok(())
}

/// DataComponentPredicate type codecs (vanilla 26.3 `DataComponentPredicates`).
pub fn decode_predicate(id: &str, t: &Tag) -> Result<(), String> {
    decode_predicate_with(id, t, &Registries::default())
}

/// DataComponentPredicate type codecs validated against the active registries.
pub fn decode_predicate_with(id: &str, t: &Tag, reg: &Registries) -> Result<(), String> {
    decode_predicate_depth(id, t, reg, 0)
}

fn decode_predicate_depth(id: &str, t: &Tag, reg: &Registries, depth: usize) -> Result<(), String> {
    if id.starts_with("exists:") {
        return match t {
            Tag::Compound(_) => Ok(()),
            _ => Err(not_map(t)),
        };
    }
    match id {
        "minecraft:count" => int_bounds(t),
        "minecraft:custom_data" => match t {
            Tag::Compound(_) => Ok(()),
            Tag::String(s) => match super::snbt::parse_fully(&s.to_string_lossy()) {
                Ok(Tag::Compound(_)) => Ok(()),
                Ok(other) => Err(not_map(&other)),
                Err(e) => Err(e.message.plain()),
            },
            _ => Err(either(not_map(t), "Not a string".into())),
        },
        "minecraft:damage" => {
            let Tag::Compound(c) = t else {
                return Err(not_map(t));
            };
            if let Some(dur) = c.get(&"durability".into()) {
                int_bounds(dur)?;
            }
            if let Some(dmg) = c.get(&"damage".into()) {
                int_bounds(dmg)?;
            }
            Ok(())
        }
        "minecraft:enchantments" | "minecraft:stored_enchantments" => {
            let entries = list_elements(t, "Not a map")?;
            for entry in entries {
                let Tag::Compound(c) = entry else {
                    return Err(not_map(entry));
                };
                if let Some(ench) = c.get(&"enchantments".into()) {
                    holder_set(ench, "minecraft:enchantment", reg)?;
                }
                if let Some(levels) = c.get(&"levels".into()) {
                    int_bounds(levels)?;
                }
            }
            Ok(())
        }
        "minecraft:potion_contents" => {
            let Tag::Compound(c) = t else {
                return Err(not_map(t));
            };
            if let Some(potions) = c.get(&"potions".into()) {
                holder_set(potions, "minecraft:potion", reg)?;
            }
            if let Some(effects) = c.get(&"effects".into()) {
                collection_predicate(effects, &|p| mob_effects_predicate(p, reg))?;
            }
            Ok(())
        }
        "minecraft:container" | "minecraft:bundle_contents" => {
            let Tag::Compound(c) = t else {
                return Err(not_map(t));
            };
            if let Some(items) = c.get(&"items".into()) {
                collection_predicate(items, &|p| item_predicate_tag(p, reg, depth + 1))?;
            }
            Ok(())
        }
        "minecraft:firework_explosion" => firework_explosion_predicate(t),
        "minecraft:fireworks" => {
            let Tag::Compound(c) = t else {
                return Err(not_map(t));
            };
            if let Some(explosions) = c.get(&"explosions".into()) {
                collection_predicate(explosions, &firework_explosion_predicate)?;
            }
            if let Some(flight) = c.get(&"flight_duration".into()) {
                int_bounds(flight)?;
            }
            Ok(())
        }
        "minecraft:writable_book_content" => {
            let Tag::Compound(c) = t else {
                return Err(not_map(t));
            };
            if let Some(pages) = c.get(&"pages".into()) {
                collection_predicate(pages, &string_value)?;
            }
            Ok(())
        }
        "minecraft:written_book_content" => {
            let Tag::Compound(c) = t else {
                return Err(not_map(t));
            };
            if let Some(pages) = c.get(&"pages".into()) {
                collection_predicate(pages, &super::component::decode)?;
            }
            if let Some(author) = c.get(&"author".into()) {
                string_value(author)?;
            }
            if let Some(title) = c.get(&"title".into()) {
                string_value(title)?;
            }
            if let Some(generation) = c.get(&"generation".into()) {
                int_bounds(generation)?;
            }
            if let Some(resolved) = c.get(&"resolved".into()) {
                bool_value(resolved)?;
            }
            Ok(())
        }
        "minecraft:attribute_modifiers" => {
            let Tag::Compound(c) = t else {
                return Err(not_map(t));
            };
            if let Some(modifiers) = c.get(&"modifiers".into()) {
                collection_predicate(modifiers, &|e| attribute_modifier_entry_predicate(e, reg))?;
            }
            Ok(())
        }
        "minecraft:trim" => {
            let Tag::Compound(c) = t else {
                return Err(not_map(t));
            };
            if let Some(material) = c.get(&"material".into()) {
                holder_set(material, "minecraft:trim_material", reg)?;
            }
            if let Some(pattern) = c.get(&"pattern".into()) {
                holder_set(pattern, "minecraft:trim_pattern", reg)?;
            }
            Ok(())
        }
        "minecraft:jukebox_playable" => {
            let Tag::Compound(c) = t else {
                return Err(not_map(t));
            };
            if let Some(song) = c.get(&"song".into()) {
                holder_set(song, "minecraft:jukebox_song", reg)?;
            }
            Ok(())
        }
        "minecraft:villager/variant" => holder_set(t, "minecraft:villager_type", reg),
        _ => Ok(()),
    }
}

// ---- ItemParser -------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum ISugg {
    Nothing,
    Item,
    StartComponents,
    ComponentAssignmentOrRemoval,
    Component,
    Assignment,
    NextOrEndComponents,
}

struct ItemState<'r, 'a> {
    reader: &'r mut Reader<'a>,
    reg: &'r Registries,
    sugg: ISugg,
}
impl ItemState<'_, '_> {
    fn parse(&mut self) -> CmdResult<ItemInput> {
        self.sugg = ISugg::Item;
        let cursor = self.reader.cursor;
        let id = read_identifier(self.reader)?;
        if !self.reg.ids("minecraft:item").contains(&id) {
            self.reader.cursor = cursor;
            return Err(CommandError::tr_at(
                "argument.item.id.invalid",
                &[&id],
                self.reader,
            ));
        }
        self.sugg = ISugg::StartComponents;
        let mut components = None;
        if self.reader.can_read() && self.reader.peek() == '[' {
            self.sugg = ISugg::Nothing;
            let start = self.reader.cursor;
            self.read_components()?;
            components = Some(self.reader.string[start..self.reader.cursor].to_owned());
        }
        Ok(ItemInput {
            id,
            tag: false,
            any: false,
            components,
        })
    }
    fn component_type(&mut self) -> CmdResult<String> {
        if !self.reader.can_read() {
            return Err(CommandError::tr_at(
                "arguments.item.component.expected",
                &[],
                self.reader,
            ));
        }
        let cursor = self.reader.cursor;
        let id = read_identifier(self.reader)?;
        if self.reg.persistent_components.contains(&id) {
            return Ok(id);
        }
        self.reader.cursor = cursor;
        Err(CommandError::tr_at(
            "arguments.item.component.unknown",
            &[&id],
            self.reader,
        ))
    }
    fn read_components(&mut self) -> CmdResult<()> {
        self.reader.expect('[')?;
        self.sugg = ISugg::ComponentAssignmentOrRemoval;
        let mut visited: Vec<String> = Vec::new();
        while self.reader.can_read() && self.reader.peek() != ']' {
            self.reader.skip_whitespace();
            if self.reader.can_read() && self.reader.peek() == '!' {
                self.reader.skip();
                self.sugg = ISugg::Component;
                let ty = self.component_type()?;
                if visited.contains(&ty) {
                    return Err(CommandError::tr(
                        "arguments.item.component.repeated",
                        &[&ty],
                    ));
                }
                visited.push(ty);
                self.sugg = ISugg::Nothing;
                self.reader.skip_whitespace();
            } else {
                let ty = self.component_type()?;
                if visited.contains(&ty) {
                    return Err(CommandError::tr(
                        "arguments.item.component.repeated",
                        &[&ty],
                    ));
                }
                visited.push(ty.clone());
                self.sugg = ISugg::Assignment;
                self.reader.skip_whitespace();
                self.reader.expect('=')?;
                self.sugg = ISugg::Nothing;
                self.reader.skip_whitespace();
                let cursor = self.reader.cursor;
                let tag = super::snbt::parse_tag_argument(self.reader)?;
                if let Err(m) = decode_component(&ty, &tag, self.reg) {
                    self.reader.cursor = cursor;
                    return Err(CommandError::tr_at(
                        "arguments.item.component.malformed",
                        &[&ty, &m],
                        self.reader,
                    ));
                }
                self.reader.skip_whitespace();
            }
            self.sugg = ISugg::NextOrEndComponents;
            if !self.reader.can_read() || self.reader.peek() != ',' {
                break;
            }
            self.reader.skip();
            self.reader.skip_whitespace();
            self.sugg = ISugg::ComponentAssignmentOrRemoval;
            if !self.reader.can_read() {
                return Err(CommandError::tr_at(
                    "arguments.item.component.expected",
                    &[],
                    self.reader,
                ));
            }
        }
        self.reader.expect(']')?;
        self.sugg = ISugg::Nothing;
        Ok(())
    }
    fn suggest_component(&self, b: &mut SuggestionsBuilder, suffix: &str) {
        let contents = b.remaining_lower();
        for id in &self.reg.persistent_components {
            let matched = if contents.contains(':') {
                super::args::matches_sub_str(&contents, id)
            } else {
                let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
                super::args::matches_sub_str(&contents, ns)
                    || super::args::matches_sub_str(&contents, path)
            };
            if matched {
                b.suggest(format!("{id}{suffix}"));
            }
        }
    }
    fn fill(&self, builder: &mut SuggestionsBuilder) {
        let mut b = builder.offset(self.reader.cursor);
        let empty = b.remaining().is_empty();
        match self.sugg {
            ISugg::Nothing => {}
            ISugg::Item => suggest_resources(&mut b, self.reg.ids("minecraft:item"), ""),
            ISugg::StartComponents => {
                if empty {
                    b.suggest("[");
                }
            }
            ISugg::ComponentAssignmentOrRemoval => {
                b.suggest("!");
                self.suggest_component(&mut b, "=");
            }
            ISugg::Component => self.suggest_component(&mut b, ""),
            ISugg::Assignment => {
                if empty {
                    b.suggest("=");
                }
            }
            ISugg::NextOrEndComponents => {
                if empty {
                    b.suggest(",");
                    b.suggest("]");
                }
            }
        }
        builder.add(b);
    }
}

/// ItemArgument.parse (ItemParser.parse; the reader is rewound on errors).
pub fn parse_stack(reader: &mut Reader, reg: &Registries) -> CmdResult<ItemInput> {
    let cursor = reader.cursor;
    let r = ItemState {
        reader,
        reg,
        sugg: ISugg::Nothing,
    }
    .parse();
    if r.is_err() {
        reader.cursor = cursor;
    }
    r
}
/// ItemParser.fillSuggestions.
pub fn suggest_stack(builder: &mut SuggestionsBuilder, reg: &Registries) {
    let mut reader = Reader {
        string: builder.input,
        cursor: builder.start,
    };
    let mut state = ItemState {
        reader: &mut reader,
        reg,
        sugg: ISugg::Nothing,
    };
    let _ = state.parse();
    state.fill(builder);
}

// ---- ItemPredicateArgument ------------------------------------------------

#[derive(Clone, Debug)]
pub enum TypeRes {
    Item(String),
    Tag(String),
}
#[derive(Clone, Debug)]
pub enum IV {
    Id(String),
    Type(Option<TypeRes>),
    Elem(TypeRes),
    Unit,
    Test,
    Comp(String),
    Pred(String),
    Tag(Tag),
    List(Vec<IV>),
}
impl Value for IV {
    fn list(items: Vec<Self>) -> Self {
        IV::List(items)
    }
}

fn reg<'a>(state: &State<'a, IV>) -> &'a Registries {
    state
        .ctx
        .and_then(|c| c.downcast_ref::<Registries>())
        .expect("item predicate grammar needs the registries")
}

/// IdentifierParseRule: skip whitespace, a non-empty identifier (no error).
fn id_rule(state: &mut State<IV>) -> Option<IV> {
    let mut r = state.reader();
    r.skip_whitespace();
    let start = r.cursor;
    while r.can_read() && is_id_char(r.peek()) {
        r.skip();
    }
    let raw = &r.string[start..r.cursor];
    let id = if raw.is_empty() {
        None
    } else {
        parse_identifier(raw)
    };
    match id {
        Some(id) => {
            state.set_reader(&r);
            Some(IV::Id(id))
        }
        None => {
            r.cursor = start;
            state.set_reader(&r);
            None
        }
    }
}

/// ResourceLookupRule: an identifier validated by `validate`.
fn lookup(
    state: &mut State<IV>,
    id_rule: usize,
    possible: Rc<Vec<String>>,
    validate: &dyn Fn(&str, &Reader) -> Result<IV, CommandError>,
) -> Option<IV> {
    let mut r = state.reader();
    r.skip_whitespace();
    state.set_reader(&r);
    let mark = state.cursor;
    match state.parse(id_rule) {
        Some(IV::Id(id)) => match validate(&id, &state.reader()) {
            Ok(v) => Some(v),
            Err(e) => {
                state.store(mark, Sugg::Resources(possible), Reason::Error(e));
                None
            }
        },
        _ => {
            state.store(
                mark,
                Sugg::Resources(possible),
                Reason::Delayed("argument.id.invalid", Vec::new()),
            );
            None
        }
    }
}

fn predicate_grammar() -> Grammar<IV> {
    let mut g: Grammar<IV> = Grammar::new();
    let id = g.put_custom("id", id_rule);
    let ty = g.named("type");
    let conditions = g.named("conditions");
    let ty2 = g.named("type");
    g.put_term(
        "top",
        alt(vec![
            seq(vec![ty, ch('['), Term::Cut, opt(conditions), ch(']')]),
            ty2,
        ]),
        |s| {
            let mut out = Vec::new();
            if let Some(t) = s.scope.get("type") {
                out.push(t);
            }
            if let Some(IV::List(c)) = s.scope.get("conditions") {
                out.extend(c);
            }
            Some(IV::List(out))
        },
    );
    let element = g.named("element_type");
    let tag = g.named("tag_type");
    let any = g.named("any_type");
    g.put_term(
        "type",
        alt(vec![element, seq(vec![ch('#'), Term::Cut, tag]), any]),
        |s| {
            Some(IV::Type(
                match s.scope.get_any(&["element_type", "tag_type"]) {
                    Some(IV::Elem(t)) => Some(t),
                    _ => None,
                },
            ))
        },
    );
    g.put_term("any_type", ch('*'), |_| Some(IV::Unit));
    g.put_custom("element_type", move |s| {
        let r = reg(s);
        let items = Rc::new(r.ids("minecraft:item").to_vec());
        lookup(s, id, items, &|id, reader| {
            if r.ids("minecraft:item").iter().any(|i| i == id) {
                Ok(IV::Elem(TypeRes::Item(id.to_owned())))
            } else {
                Err(CommandError::tr_at(
                    "argument.item.id.invalid",
                    &[&id],
                    reader,
                ))
            }
        })
    });
    g.put_custom("tag_type", move |s| {
        let r = reg(s);
        let tags = Rc::new(r.tags("minecraft:item").to_vec());
        lookup(s, id, tags, &|id, reader| {
            if r.tags("minecraft:item").iter().any(|i| i == id) {
                Ok(IV::Elem(TypeRes::Tag(id.to_owned())))
            } else {
                Err(CommandError::tr_at(
                    "arguments.item.tag.unknown",
                    &[&id],
                    reader,
                ))
            }
        })
    });
    let alternatives = g.named("alternatives");
    let rest = g.named("conditions");
    g.put_term(
        "conditions",
        seq(vec![alternatives, opt(seq(vec![ch(','), rest]))]),
        |s| {
            let mut out = vec![IV::Test];
            if let Some(IV::List(c)) = s.scope.get("conditions") {
                out.extend(c);
            }
            Some(IV::List(out))
        },
    );
    let term = g.named("term");
    let rest = g.named("alternatives");
    g.put_term(
        "alternatives",
        seq(vec![term, opt(seq(vec![ch('|'), rest]))]),
        |s| {
            let mut out = vec![s.scope.get("term").unwrap_or(IV::Test)];
            if let Some(IV::List(c)) = s.scope.get("alternatives") {
                out.extend(c);
            }
            Some(IV::List(out))
        },
    );
    let test = g.named("test");
    let negation = g.named("negation");
    g.put_term("term", alt(vec![test, seq(vec![ch('!'), negation])]), |s| {
        s.scope.get_any(&["test", "negation"])
    });
    let test = g.named("test");
    g.put_term("negation", test, |_| Some(IV::Test));
    let comp = g.named("component_type");
    let tag_a = g.named("tag");
    let pred = g.named("predicate_type");
    let tag_b = g.named("tag");
    let comp2 = g.named("component_type");
    g.put_term(
        "test",
        alt(vec![
            seq(vec![comp, ch('='), Term::Cut, tag_a]),
            seq(vec![pred, ch('~'), Term::Cut, tag_b]),
            comp2,
        ]),
        |s| {
            let value = match s.scope.get("tag") {
                Some(IV::Tag(t)) => Some(t),
                _ => None,
            };
            let reader = s.reader();
            let result = if let Some(IV::Pred(p)) = s.scope.get("predicate_type") {
                decode_predicate_with(&p, value.as_ref().unwrap_or(&Tag::End), reg(s)).map_err(
                    |m| {
                        let display = p.strip_prefix("exists:").unwrap_or(&p);
                        CommandError::tr_at(
                            "arguments.item.predicate.malformed",
                            &[&display, &m],
                            &reader,
                        )
                    },
                )
            } else if let (Some(IV::Comp(c)), Some(v)) = (s.scope.get("component_type"), value) {
                decode_component(&c, &v, reg(s)).map_err(|m| {
                    CommandError::tr_at("arguments.item.component.malformed", &[&c, &m], &reader)
                })
            } else {
                Ok(())
            };
            match result {
                Ok(()) => Some(IV::Test),
                Err(e) => {
                    s.store(s.cursor, Sugg::Empty, Reason::Error(e));
                    None
                }
            }
        },
    );
    g.put_custom("component_type", move |s| {
        let r = reg(s);
        let mut list = vec!["minecraft:count".to_owned()];
        list.extend(r.persistent_components.iter().cloned());
        lookup(s, id, Rc::new(list), &|id, reader| {
            if id == "minecraft:count" {
                return Ok(IV::Comp(id.to_owned()));
            }
            let known = r
                .ids("minecraft:data_component_type")
                .iter()
                .any(|c| c == id);
            if known && r.persistent_components.iter().any(|c| c == id) {
                Ok(IV::Comp(id.to_owned()))
            } else {
                Err(CommandError::tr_at(
                    "arguments.item.component.unknown",
                    &[&id],
                    reader,
                ))
            }
        })
    });
    g.put_custom("predicate_type", move |s| {
        let r = reg(s);
        let mut list = vec!["minecraft:count".to_owned()];
        list.extend(
            r.ids("minecraft:data_component_predicate_type")
                .iter()
                .cloned(),
        );
        lookup(s, id, Rc::new(list), &|id, reader| {
            if id == "minecraft:count"
                || r.ids("minecraft:data_component_predicate_type")
                    .iter()
                    .any(|c| c == id)
            {
                return Ok(IV::Pred(id.to_owned()));
            }
            if r.ids("minecraft:data_component_type")
                .iter()
                .any(|c| c == id)
            {
                // A component existence predicate (decodes Unit).
                return Ok(IV::Pred(format!("exists:{id}")));
            }
            Err(CommandError::tr_at(
                "arguments.item.predicate.unknown",
                &[&id],
                reader,
            ))
        })
    });
    g.put_custom("tag", |s| super::snbt::rule_in(s, IV::Tag));
    g.top = g.id("top");
    g
}

thread_local! {
    static PREDICATE: Grammar<IV> = predicate_grammar();
}
/// ItemPredicateArgument.parse.
pub fn parse_predicate(reader: &mut Reader, reg: &Registries) -> CmdResult<ItemInput> {
    let start = reader.cursor;
    let result = PREDICATE.with(|g| g.parse_for_commands_with(reader, Some(reg)))?;
    let IV::List(items) = result else {
        return Err(CommandError::tr_at("argument.id.invalid", &[], reader));
    };
    let text = &reader.string[start..reader.cursor];
    let (id, tag, any) = match items.first() {
        Some(IV::Type(Some(TypeRes::Item(i)))) => (i.clone(), false, false),
        Some(IV::Type(Some(TypeRes::Tag(t)))) => (t.clone(), true, false),
        _ => ("*".to_owned(), false, true),
    };
    let components = text.find('[').map(|i| text[i..].to_owned());
    Ok(ItemInput {
        id,
        tag,
        any,
        components,
    })
}
/// ItemPredicateArgument.listSuggestions.
pub fn suggest_predicate(builder: &mut SuggestionsBuilder, reg: &Registries) {
    PREDICATE.with(|g| g.parse_for_suggestions_with(builder, Some(reg)));
}

/// `LodestoneTracker.CODEC`: optional GlobalPos and a strict optional boolean.
fn lodestone_tracker(value: &Tag) -> Result<Tag, String> {
    let tracker = horizon_items::item_data::ingredient_components::lodestone_tracker(value)?;
    let mut fields = Compound::new();
    if let Some((dimension, pos)) = tracker.target {
        // ResourceKey.codec uses Identifier.CODEC, not a dimension registry lookup.
        let target = Tag::compound([
            ("dimension", identifier_value(dimension)?),
            ("pos", Tag::IntArray(pos.to_vec())),
        ]);
        fields.insert("target".into(), target);
    }
    if !tracker.tracked {
        fields.insert("tracked".into(), Tag::Byte(0));
    }
    Ok(Tag::Compound(fields))
}

/// `TooltipDisplay.CODEC` and `DataComponentType.CODEC`, using the live registry.
fn tooltip_display(value: &Tag, reg: &Registries) -> Result<Tag, String> {
    let display = horizon_items::properties::tooltip_display::TooltipDisplay::parse(value)
        .ok_or("Invalid tooltip display")?;
    let mut hidden = Vec::new();
    for id in display.components() {
        let tag = id.to_tag();
        let Tag::String(key) = &tag else {
            unreachable!("identifier tag")
        };
        let key = key.to_string_lossy();
        if !reg.ids("minecraft:data_component_type").contains(&key)
            && !reg.persistent_components.contains(&key)
            && horizon_protocol::item::custom::id(&key).is_none()
        {
            return Err(format!("Unknown data component type: {key}"));
        }
        if !hidden.contains(&tag) {
            hidden.push(tag);
        }
    }
    Ok(Tag::compound([
        ("hide_tooltip", Tag::Byte(i8::from(display.hide_tooltip))),
        ("hidden_components", Tag::List(hidden)),
    ]))
}

/// `SeededContainerLoot.CODEC`: a resource key (not a registry lookup) and a long.
fn container_loot(value: &Tag) -> Result<Tag, String> {
    let (table, seed) =
        horizon_items::item_data::ingredient_components::container_loot_fields(value)?;
    Ok(Tag::compound([
        ("loot_table", identifier_value(table)?),
        ("seed", Tag::Long(seed)),
    ]))
}

#[cfg(test)]
mod tests;
