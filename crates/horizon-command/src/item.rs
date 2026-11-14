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
    if id == "minecraft:item_model" {
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
        "minecraft:max_stack_size" => {
            let v = int_of(t).ok_or("Not a number")?;
            if !(1..=99).contains(&v) {
                return Err(format!("Value must be within range [1;99]: {v}"));
            }
            Ok(())
        }
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
        "minecraft:enchantments" | "minecraft:stored_enchantments" => enchantment_map(t, reg),
        "minecraft:tooltip_display" => tooltip_display(t, reg).map(|_| ()),
        // DataComponents.ENCHANTMENT_GLINT_OVERRIDE: Codec.BOOL through NbtOps.
        "minecraft:enchantment_glint_override" => horizon_items::item_data::component_bool(t)
            .map(|_| ())
            .ok_or_else(|| "Not a number".into()),
        "minecraft:custom_name" | "minecraft:item_name" => super::component::decode(t),
        "minecraft:item_model" => identifier_value(t).map(|_| ()),
        "minecraft:damage_resistant" => component_values::damage_resistant(t, reg).map(|_| ()),
        "minecraft:use_effects" => horizon_items::item_data::ingredient_components::use_effects(t)
            .map(|_| ())
            .map_err(str::to_owned),
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
                Some(_) => Ok(()),
            },
            _ => Ok(()),
        },
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

/// MinMaxBounds.Ints.CODEC.
fn int_bounds(t: &Tag) -> Result<(), String> {
    match t {
        Tag::Compound(_) => Ok(()),
        _ if is_number(t) => Ok(()),
        _ => Err(either(not_map(t), "Not a number".into())),
    }
}
/// DataComponentPredicate type codecs.
pub fn decode_predicate(id: &str, t: &Tag) -> Result<(), String> {
    match id {
        "minecraft:count" => int_bounds(t),
        "minecraft:damage" => match t {
            Tag::Compound(_) => Ok(()),
            _ => Err(not_map(t)),
        },
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
                decode_predicate(&p, value.as_ref().unwrap_or(&Tag::End)).map_err(|m| {
                    CommandError::tr_at("arguments.item.predicate.malformed", &[&p, &m], &reader)
                })
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
mod tests {
    #[test]
    fn pet_variant_holders_validate_command_assignment_and_equality_predicates() {
        use super::{Reader, Registries, parse_predicate, parse_stack};
        use crate::args::RegistryIds;
        let mut reg = Registries {
            items: vec!["minecraft:stone".into()],
            ..Default::default()
        };
        for (component, registry) in [
            ("minecraft:cat/sound_variant", "minecraft:cat_sound_variant"),
            ("minecraft:cat/variant", "minecraft:cat_variant"),
            (
                "minecraft:wolf/sound_variant",
                "minecraft:wolf_sound_variant",
            ),
            ("minecraft:wolf/variant", "minecraft:wolf_variant"),
        ] {
            reg.persistent_components.push(component.into());
            reg.by_registry.insert(
                registry.into(),
                RegistryIds {
                    ids: vec!["example:custom/pet".into()],
                    tags: vec!["example:pets".into()],
                },
            );
        }
        reg.by_registry.insert(
            "minecraft:data_component_type".into(),
            RegistryIds {
                ids: reg.persistent_components.clone(),
                tags: vec![],
            },
        );
        for component in &reg.persistent_components {
            for parser in [parse_stack, parse_predicate] {
                let input = format!("stone[{component}=\"example:custom/pet\"]");
                let mut reader = Reader::new(&input);
                assert!(parser(&mut reader, &reg).is_ok(), "{input}");
                assert_eq!(reader.cursor, input.len());
                for value in [
                    "\"example:missing\"",
                    "\"#example:pets\"",
                    "{id:\"example:custom/pet\"}",
                    "1",
                ] {
                    let input = format!("stone[{component}={value}]");
                    assert!(parser(&mut Reader::new(&input), &reg).is_err(), "{input}");
                }
                let input = format!("stone[!{component}]");
                assert!(parser(&mut Reader::new(&input), &reg).is_ok(), "{input}");
            }
        }
    }

    #[test]
    fn pet_variant_holders_use_the_supplied_registry_and_normalize_identifiers() {
        use super::{Registries, Tag, decoded_component_value};
        use crate::args::RegistryIds;
        use horizon_core::nbt::JavaString;

        // DataComponents -> each variant CODEC -> RegistryFixedCodec, not DIRECT_CODEC.
        for (component, registry, builtin) in [
            (
                "minecraft:cat/sound_variant",
                "minecraft:cat_sound_variant",
                "classic",
            ),
            ("minecraft:cat/variant", "minecraft:cat_variant", "tabby"),
            (
                "minecraft:wolf/sound_variant",
                "minecraft:wolf_sound_variant",
                "classic",
            ),
            ("minecraft:wolf/variant", "minecraft:wolf_variant", "pale"),
        ] {
            let mut reg = Registries::default();
            assert!(
                decoded_component_value(component, &Tag::String(builtin.into()), &reg).is_err()
            );
            reg.by_registry.insert(
                registry.into(),
                RegistryIds {
                    ids: vec![
                        format!("minecraft:{builtin}"),
                        "example:custom/pet".into(),
                        "minecraft:".into(),
                    ],
                    tags: vec!["example:known_tag".into()],
                },
            );
            for raw in [
                builtin.to_owned(),
                format!(":{builtin}"),
                format!("minecraft:{builtin}"),
            ] {
                assert_eq!(
                    decoded_component_value(component, &Tag::String(raw.as_str().into()), &reg)
                        .unwrap(),
                    Tag::String(format!("minecraft:{builtin}").as_str().into()),
                );
            }
            assert_eq!(
                decoded_component_value(component, &Tag::String("example:custom/pet".into()), &reg)
                    .unwrap(),
                Tag::String("example:custom/pet".into()),
            );
            // Identifier accepts an empty path, but it must still resolve in the snapshot.
            assert_eq!(
                decoded_component_value(component, &Tag::String("".into()), &reg).unwrap(),
                Tag::String("minecraft:".into()),
            );
            for value in [
                Tag::String("example:missing".into()),
                Tag::String("example:known_tag".into()),
                Tag::String("#example:known_tag".into()),
                Tag::String("EXAMPLE:custom/pet".into()),
                Tag::String("example:Custom/pet".into()),
                Tag::String("..:custom/pet".into()),
                Tag::String("example:custom:pet".into()),
                Tag::String(JavaString(vec![0xd800])),
                Tag::String("é".into()),
                Tag::Int(1),
                Tag::Float(f32::NAN.to_bits()),
                Tag::List(vec![Tag::String(builtin.into())]),
                Tag::compound([("id", Tag::String(builtin.into()))]),
                Tag::compound([("asset_id", Tag::String("entity/cat/cat_tabby".into()))]),
            ] {
                assert!(
                    decoded_component_value(component, &value, &reg).is_err(),
                    "{component}: {value:?}"
                );
                assert!(decode_component(component, &value, &reg).is_err());
            }
            reg.by_registry.get_mut(registry).unwrap().ids.clear();
            assert!(decode_component(component, &Tag::String(builtin.into()), &reg).is_err());
        }
    }

    #[test]
    fn cushion_color_codec_rejects_non_string_and_noncanonical_names() {
        use super::{Registries, Tag, decoded_component_value};
        use horizon_core::nbt::JavaString;
        let registry = Registries::default();
        assert!(super::DECODED_COMPONENTS.contains(&"minecraft:cushion/color"));
        for value in [
            Tag::End,
            Tag::String("RED".into()),
            Tag::String("silver".into()),
            Tag::String("minecraft:red".into()),
            Tag::String(" red".into()),
            Tag::String("".into()),
            Tag::String(JavaString(vec![0xd800])),
            Tag::String(JavaString(vec![114, 101, 100, 0xd800])),
            Tag::Byte(14),
            Tag::Short(14),
            Tag::Long(14),
            Tag::Float(14.0f32.to_bits()),
            Tag::Double(14.0f64.to_bits()),
            Tag::List(vec![]),
            Tag::Compound(Default::default()),
            Tag::ByteArray(vec![]),
            Tag::IntArray(vec![]),
            Tag::LongArray(vec![]),
        ] {
            assert!(decoded_component_value("minecraft:cushion/color", &value, &registry).is_err());
        }
        for ordinal in -1..=16 {
            assert!(
                decoded_component_value("minecraft:cushion/color", &Tag::Int(ordinal), &registry)
                    .is_err()
            );
        }
        let value = Tag::String("light_gray".into());
        assert_eq!(
            decoded_component_value("minecraft:cushion/color", &value, &registry).unwrap(),
            value
        );
    }

    #[test]
    fn tooltip_display_decodes_complete_persistent_nbt_codec() {
        use super::{Registries, Tag, decoded_component_value};
        let mut registry = Registries::default();
        registry
            .by_registry
            .entry("minecraft:data_component_type".into())
            .or_default()
            .ids = vec!["minecraft:damage".into(), "fixture:transient".into()];
        registry
            .persistent_components
            .push("fixture:declared".into());
        let decode =
            |tag: &Tag| decoded_component_value("minecraft:tooltip_display", tag, &registry);
        for (number, expected) in [
            (Tag::Byte(-1), 1),
            (Tag::Short(0), 0),
            (Tag::Int(2), 1),
            (Tag::Long(1_i64 << 32), 1),
            (Tag::Float(0.5_f32.to_bits()), 1),
            (Tag::Float((-0.0_f32).to_bits()), 0),
            (Tag::Float(f32::NAN.to_bits()), 1),
            (Tag::Double(f64::MIN_POSITIVE.to_bits()), 1),
            (Tag::Double((-0.0_f64).to_bits()), 0),
            (Tag::Double(f64::NAN.to_bits()), 1),
        ] {
            assert_eq!(
                decode(&Tag::compound([("hide_tooltip", number)])).unwrap(),
                Tag::compound([
                    ("hide_tooltip", Tag::Byte(expected)),
                    ("hidden_components", Tag::List(vec![]))
                ])
            );
        }
        let expected = Tag::compound([
            ("hide_tooltip", Tag::Byte(0)),
            (
                "hidden_components",
                Tag::List(vec![
                    Tag::String("minecraft:damage".into()),
                    Tag::String("fixture:declared".into()),
                    Tag::String("fixture:transient".into()),
                ]),
            ),
        ]);
        assert_eq!(
            decode(&Tag::compound([
                (
                    "hidden_components",
                    Tag::List(vec![
                        Tag::String("damage".into()),
                        Tag::String(":damage".into()),
                        Tag::String("fixture:declared".into()),
                        Tag::String("fixture:transient".into()),
                    ])
                ),
                (
                    "ignored",
                    Tag::String("record codecs ignore unknown fields".into())
                ),
            ]))
            .unwrap(),
            expected
        );
        for empty in [
            Tag::List(vec![]),
            Tag::ByteArray(vec![]),
            Tag::IntArray(vec![]),
            Tag::LongArray(vec![]),
        ] {
            assert_eq!(
                decode(&Tag::compound([("hidden_components", empty)])).unwrap(),
                Tag::compound([
                    ("hide_tooltip", Tag::Byte(0)),
                    ("hidden_components", Tag::List(vec![]))
                ])
            );
        }
        for invalid in [
            Tag::Byte(0),
            Tag::compound([("hide_tooltip", Tag::String("true".into()))]),
            Tag::compound([("hide_tooltip", Tag::Compound(Default::default()))]),
            Tag::compound([("hidden_components", Tag::String("damage".into()))]),
            Tag::compound([("hidden_components", Tag::List(vec![Tag::Int(1)]))]),
            Tag::compound([("hidden_components", Tag::ByteArray(vec![1]))]),
            Tag::compound([("hidden_components", Tag::IntArray(vec![1]))]),
            Tag::compound([("hidden_components", Tag::LongArray(vec![1]))]),
            Tag::compound([(
                "hidden_components",
                Tag::List(vec![Tag::String("fixture:unknown".into())]),
            )]),
            Tag::compound([(
                "hidden_components",
                Tag::List(vec![Tag::String("Bad:identifier".into())]),
            )]),
        ] {
            assert!(decode(&invalid).is_err(), "{invalid:?}");
            assert!(decode_component("minecraft:tooltip_display", &invalid, &registry).is_err());
        }
    }

    #[test]
    fn dye_codec_accepts_only_exact_names_under_ordinary_nbt_ops() {
        use super::{Registries, Tag, decoded_component_value};
        let registry = Registries::default();
        // Literal oracle names, independent of the palette used by the decoder.
        for name in [
            "white",
            "orange",
            "magenta",
            "light_blue",
            "yellow",
            "lime",
            "pink",
            "gray",
            "light_gray",
            "cyan",
            "purple",
            "blue",
            "brown",
            "green",
            "red",
            "black",
        ] {
            let value = Tag::String(name.into());
            assert_eq!(
                decoded_component_value("minecraft:dye", &value, &registry).unwrap(),
                value
            );
            assert!(
                decode_component(
                    "minecraft:dye",
                    &Tag::String(name.to_uppercase().as_str().into()),
                    &registry
                )
                .is_err()
            );
        }
        for value in [
            Tag::String("silver".into()),
            Tag::String("minecraft:red".into()),
            Tag::String(" red".into()),
            Tag::String("".into()),
            Tag::Byte(0),
            Tag::Byte(14),
            Tag::Short(14),
            Tag::Int(14),
            Tag::Long(14),
            Tag::Float(14.0f32.to_bits()),
            Tag::Double(14.0f64.to_bits()),
            Tag::List(vec![]),
            Tag::Compound(Default::default()),
        ] {
            assert!(decoded_component_value("minecraft:dye", &value, &registry).is_err());
        }
        // File compression does not enable DynamicOps.compressMaps(): even valid
        // dye ordinals are not persistent NbtOps names.
        for ordinal in -1..=16 {
            assert!(decode_component("minecraft:dye", &Tag::Int(ordinal), &registry).is_err());
        }
    }

    #[test]
    fn dye_codec_rejects_non_names_without_lossy_utf16_conversion() {
        use super::{Registries, Tag, decoded_component_value};
        use horizon_core::nbt::JavaString;
        let registry = Registries::default();
        for value in [
            Tag::End,
            Tag::ByteArray(vec![]),
            Tag::IntArray(vec![]),
            Tag::LongArray(vec![]),
            Tag::String(JavaString(vec![0xd800])),
            Tag::String(JavaString(vec![114, 101, 100, 0xd800])),
        ] {
            assert!(decoded_component_value("minecraft:dye", &value, &registry).is_err());
        }
    }

    #[test]
    fn weapon_codec_defaults_numeric_conversions_and_ranges() {
        use super::{Registries, Tag, decoded_component_value};
        let registry = Registries::default();
        let decode = |tag: &Tag| decoded_component_value("minecraft:weapon", tag, &registry);
        let canonical = |damage, seconds: f32| {
            Tag::compound([
                ("item_damage_per_attack", Tag::Int(damage)),
                (
                    "disable_blocking_for_seconds",
                    Tag::Float(seconds.to_bits()),
                ),
            ])
        };
        assert_eq!(
            decode(&Tag::Compound(Default::default())).unwrap(),
            canonical(1, 0.0)
        );
        for (number, expected) in [
            (Tag::Byte(2), 2),
            (Tag::Short(2), 2),
            (Tag::Long(4_294_967_298), 2),
            (Tag::Double(2.75f64.to_bits()), 2),
            (Tag::Float(f32::NAN.to_bits()), 0),
            (Tag::Double(f64::INFINITY.to_bits()), i32::MAX),
        ] {
            assert_eq!(
                decode(&Tag::compound([
                    ("item_damage_per_attack", number),
                    (
                        "disable_blocking_for_seconds",
                        Tag::Double(1.25f64.to_bits())
                    ),
                    ("ignored", Tag::String("anything".into())),
                ]))
                .unwrap(),
                canonical(expected, 1.25)
            );
        }
        for number in [
            Tag::Byte(-1),
            Tag::Long(2_147_483_648),
            Tag::String("1".into()),
        ] {
            assert!(decode(&Tag::compound([("item_damage_per_attack", number)])).is_err());
        }
        for number in [
            Tag::Float((-0.0f32).to_bits()),
            Tag::Float((-1.0f32).to_bits()),
            Tag::Float(f32::NAN.to_bits()),
            Tag::Float(f32::INFINITY.to_bits()),
            Tag::Float(f32::NEG_INFINITY.to_bits()),
            Tag::Double(f64::MAX.to_bits()),
            Tag::String("0".into()),
        ] {
            assert!(decode(&Tag::compound([("disable_blocking_for_seconds", number)])).is_err());
        }
        assert_eq!(
            decode(&Tag::compound([(
                "disable_blocking_for_seconds",
                Tag::Float(f32::MAX.to_bits()),
            )]))
            .unwrap(),
            canonical(1, f32::MAX)
        );
        assert!(decode(&Tag::List(Vec::new())).is_err());
    }

    use super::decode_component;

    #[test]
    fn fox_variant_codec_accepts_only_exact_persistent_names() {
        use super::{Registries, Tag, decoded_component_value};
        use horizon_core::nbt::JavaString;
        let registry = Registries::default();
        for name in ["red", "snow"] {
            let value = Tag::String(name.into());
            assert_eq!(
                decoded_component_value("minecraft:fox/variant", &value, &registry),
                Ok(value)
            );
        }
        for value in [
            Tag::Byte(0),
            Tag::Int(1),
            Tag::Double(1.0f64.to_bits()),
            Tag::List(vec![]),
            Tag::compound([] as [(&str, Tag); 0]),
        ] {
            assert_eq!(
                decoded_component_value("minecraft:fox/variant", &value, &registry),
                Err("Not a string".into())
            );
        }
        for name in ["", "RED", "Snow", "minecraft:red", "arctic", "red "] {
            assert_eq!(
                decoded_component_value(
                    "minecraft:fox/variant",
                    &Tag::String(name.into()),
                    &registry
                ),
                Err(format!("Unknown element name:{name}"))
            );
        }
        for units in [vec![0xd800], vec![114, 101, 100, 0xdc00]] {
            assert!(
                decoded_component_value(
                    "minecraft:fox/variant",
                    &Tag::String(JavaString(units)),
                    &registry
                )
                .is_err()
            );
        }
    }

    #[test]
    fn decoded_component_values_use_codec_numbers_and_retain_stored_values() {
        use super::{Registries, Tag, decoded_component_value};
        let registry = Registries::default();
        assert_eq!(
            decoded_component_value(
                "minecraft:damage",
                &Tag::Double(5.0f64.to_bits()),
                &registry
            )
            .unwrap(),
            Tag::Int(5)
        );
        assert_eq!(
            decoded_component_value(
                "minecraft:repair_cost",
                &Tag::Float(2.0f32.to_bits()),
                &registry
            )
            .unwrap(),
            Tag::Int(2)
        );
        assert!(
            decoded_component_value(
                "minecraft:damage",
                &Tag::String("not a number".into()),
                &registry
            )
            .is_err()
        );
        assert_eq!(
            decoded_component_value(
                "minecraft:unbreakable",
                &Tag::compound([] as [(&str, Tag); 0]),
                &registry
            )
            .unwrap(),
            Tag::compound([] as [(&str, Tag); 0])
        );
        assert!(
            decoded_component_value(
                "minecraft:block_entity_data",
                &Tag::String("invalid".into()),
                &registry
            )
            .is_err()
        );
    }

    #[test]
    fn enchantment_maps_validate_entries_against_the_supplied_registry() {
        use super::{Reader, Registries, parse_predicate, parse_stack};
        use crate::args::RegistryIds;
        let mut reg = Registries {
            items: vec!["minecraft:diamond_sword".into()],
            persistent_components: vec![
                "minecraft:enchantments".into(),
                "minecraft:stored_enchantments".into(),
            ],
            ..Default::default()
        };
        reg.by_registry.insert(
            "minecraft:data_component_type".into(),
            RegistryIds {
                ids: reg.persistent_components.clone(),
                tags: vec![],
            },
        );
        reg.by_registry.insert(
            "minecraft:enchantment".into(),
            RegistryIds {
                ids: vec!["minecraft:sharpness".into()],
                tags: vec![],
            },
        );
        for component in ["enchantments", "stored_enchantments"] {
            for value in [
                "{}",
                "{sharpness:1}",
                "{sharpness:255}",
                "{sharpness:1.5f}",
                "{sharpness:1b}",
                "{sharpness:4294967297L}",
                "{sharpness:9007199254740993L}",
                "{\"minecraft:sharpness\":1}",
            ] {
                let input = format!("diamond_sword[{component}={value}]");
                for parser in [parse_stack, parse_predicate] {
                    let mut reader = Reader::new(&input);
                    assert!(parser(&mut reader, &reg).is_ok(), "{input}");
                    assert_eq!(reader.cursor, input.len());
                }
            }
            for value in [
                "{sharpness:-1}",
                "{sharpness:0}",
                "{sharpness:256}",
                "{sharpness:100000}",
                "{sharpness:-0.5d}",
                "{sharpness:\"1\"}",
                "{levels:{sharpness:1}}",
                "{\"audit:missing\":1}",
            ] {
                let input = format!("diamond_sword[{component}={value}]");
                for parser in [parse_stack, parse_predicate] {
                    assert!(
                        parser(&mut Reader::new(&input), &reg).is_err(),
                        "{input} crossed command parsing"
                    );
                }
            }
        }
    }

    fn check(snbt: &str) -> Result<(), String> {
        let tag = horizon_core::nbt::parse_snbt(snbt).expect("snbt").0;
        decode_component(
            "minecraft:block_entity_data",
            &tag,
            &super::Registries::default(),
        )
    }

    #[test]
    fn block_entity_data_needs_a_known_type() {
        assert_eq!(check("{id:\"chest\",Lock:1}"), Ok(()));
        assert_eq!(
            check("{Command:\"say\"}"),
            Err("Expected 'id' field in {Command:\"say\"}".into())
        );
        assert_eq!(
            check("{id:\"minecraft:stone\"}"),
            Err("Unknown registry key in ResourceKey[minecraft:root / minecraft:block_entity_type]: minecraft:stone".into())
        );
    }

    #[test]
    #[ignore = "daily"]
    fn block_entity_data_takes_a_type_a_mod_declared() {
        // The declared types are frozen once per process: install them in a fresh one.
        const CHILD: &str = "HORIZON_TEST_DECLARED_BLOCK_ENTITY_DATA";
        if std::env::var_os(CHILD).is_none() {
            assert!(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--include-ignored",
                        "--exact",
                        "item::tests::block_entity_data_takes_a_type_a_mod_declared",
                        "--nocapture"
                    ])
                    .env(CHILD, "1")
                    .status()
                    .unwrap()
                    .success()
            );
            return;
        }
        use horizon_client_core::block_entity_types::{self, BlockEntityTypes};
        block_entity_types::install(BlockEntityTypes::new(
            [("example:crate".into(), "example:crate".into())].into(),
            vec!["example:crate".into()],
        ))
        .unwrap();
        assert_eq!(check("{id:\"example:crate\",Tank:1}"), Ok(()));
        assert_eq!(
            check("{id:\"example:nope\"}"),
            Err("Unknown registry key in ResourceKey[minecraft:root / minecraft:block_entity_type]: example:nope".into())
        );
    }
}
