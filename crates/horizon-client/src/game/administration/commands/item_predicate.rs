//! `ItemPredicateArgument` evaluation: exact component tests (`=`), all 15
//! vanilla 26.3 `DataComponentPredicates` (`~`), component existence (`comp`
//! and `comp~{}`), `count` pseudo-component/predicate, and nested
//! `ItemPredicate` matching (`container~`, `bundle_contents~`).

use super::{Game, split_top_level};
use horizon_client::{
    command::{
        args::ItemInput,
        item::{DECODED_COMPONENTS, decoded_component_value},
        snbt::parse_fully,
    },
    item_data::{
        component_bool, components_tag, decode_components, effective_components,
        ingredient_components::component_equal, same_component_value,
    },
    items::{ItemStack, Items},
    nbt_path::compare_nbt,
};
use horizon_client_core::registries::Registries;
use horizon_core::nbt::{Compound, JavaString, Tag};
use horizon_items::{item_nbt::item_from_nbt, properties::swing_animation::SwingAnimation};

const MAX_NESTING_DEPTH: usize = 16;

/// Borrowed item catalog, command registries, and tag resolver for evaluating
/// item predicates.
pub(super) struct PredicateEnv<'a> {
    items: &'a Items,
    registries: &'a Registries,
    tag_lookup: &'a dyn Fn(&str, &str) -> Vec<String>,
}

impl<'a> PredicateEnv<'a> {
    fn from_game(game: &'a Game<'_>, tag_lookup: &'a dyn Fn(&str, &str) -> Vec<String>) -> Self {
        Self {
            items: game.items(),
            registries: &game.ui.registries,
            tag_lookup,
        }
    }

    fn item_matches(&self, predicate: &ItemInput, item: &str) -> bool {
        if predicate.any {
            return true;
        }
        if predicate.tag {
            let tag = predicate.id.trim_start_matches("minecraft:");
            return self
                .items
                .item_tags
                .get(tag)
                .or_else(|| self.items.item_tags.get(&predicate.id))
                .is_some_and(|s| {
                    s.contains(item) || s.contains(item.trim_start_matches("minecraft:"))
                });
        }
        predicate.id == item
    }
}

fn canonical_id(raw: &str) -> String {
    horizon_core::identifier::Identifier::parse(raw)
        .map(|id| id.to_string())
        .unwrap_or_else(|_| {
            if raw.contains(':') {
                raw.to_owned()
            } else {
                format!("minecraft:{raw}")
            }
        })
}

fn tag_str(tag: &Tag) -> Option<String> {
    match tag {
        Tag::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

/// Java `Number.intValue` conversion used by `Codec.INT` via `NbtOps`.
fn int_of(tag: &Tag) -> Option<i64> {
    Some(match tag {
        Tag::Byte(v) => i64::from(*v),
        Tag::Short(v) => i64::from(*v),
        Tag::Int(v) => i64::from(*v),
        Tag::Long(v) => i64::from(*v as i32),
        Tag::Float(bits) => i64::from(f32::from_bits(*bits) as i32),
        Tag::Double(bits) => i64::from(f64::from_bits(*bits) as i32),
        _ => return None,
    })
}

/// Vanilla `ExtraCodecs.UNSIGNED_BYTE` (`0..=255`).
/// Horizon serializes `u8` fields like `flight_duration` as `Tag::Byte(v as i8)`,
/// so `128..=255` arrives as negative `i8` bytes and must be read unsigned (`*b as u8`).
fn unsigned_byte_of(tag: &Tag) -> Option<u8> {
    match tag {
        Tag::Byte(b) => Some(*b as u8),
        other => u8::try_from(int_of(other)?).ok(),
    }
}

/// Java `Number.doubleValue` conversion used by `Codec.DOUBLE` via `NbtOps`.
fn double_of(tag: &Tag) -> Option<f64> {
    Some(match tag {
        Tag::Byte(v) => f64::from(*v),
        Tag::Short(v) => f64::from(*v),
        Tag::Int(v) => f64::from(*v),
        Tag::Long(v) => *v as f64,
        Tag::Float(bits) => f64::from(f32::from_bits(*bits)),
        Tag::Double(bits) => f64::from_bits(*bits),
        _ => return None,
    })
}

/// Java `Number.floatValue` conversion used by `Codec.FLOAT` via `NbtOps`.
fn float_of(tag: &Tag) -> Option<f32> {
    Some(match tag {
        Tag::Byte(v) => f32::from(*v),
        Tag::Short(v) => f32::from(*v),
        Tag::Int(v) => *v as f32,
        Tag::Long(v) => *v as f32,
        Tag::Float(bits) => f32::from_bits(*bits),
        Tag::Double(bits) => f64::from_bits(*bits) as f32,
        _ => return None,
    })
}

fn f32_bits_key(v: f32) -> u32 {
    if v.is_nan() { 0x7fc0_0000 } else { v.to_bits() }
}

fn f64_bits_key(v: f64) -> u64 {
    if v.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        v.to_bits()
    }
}

/// `MinMaxBounds.Ints.matches`: exact numeric tag or `{min?, max?}`.
pub(super) fn matches_int_bounds(value: i64, bounds: &Tag) -> bool {
    match bounds {
        Tag::Compound(c) => {
            let min = match c.get(&"min".into()) {
                Some(v) => match int_of(v) {
                    Some(n) => Some(n),
                    None => return false,
                },
                None => None,
            };
            let max = match c.get(&"max".into()) {
                Some(v) => match int_of(v) {
                    Some(n) => Some(n),
                    None => return false,
                },
                None => None,
            };
            if let (Some(lo), Some(hi)) = (min, max)
                && lo > hi
            {
                return false;
            }
            min.is_none_or(|lo| value >= lo) && max.is_none_or(|hi| value <= hi)
        }
        other => int_of(other).is_some_and(|exact| value == exact),
    }
}

fn is_any_int_bounds(bounds: &Tag) -> bool {
    matches!(bounds, Tag::Compound(c) if c.get(&"min".into()).is_none() && c.get(&"max".into()).is_none())
}

/// `MinMaxBounds.Doubles.matches`: exact numeric tag or `{min?, max?}`.
fn matches_double_bounds(value: f64, bounds: &Tag) -> bool {
    match bounds {
        Tag::Compound(c) => {
            let min = match c.get(&"min".into()) {
                Some(v) => match double_of(v) {
                    Some(n) => Some(n),
                    None => return false,
                },
                None => None,
            };
            let max = match c.get(&"max".into()) {
                Some(v) => match double_of(v) {
                    Some(n) => Some(n),
                    None => return false,
                },
                None => None,
            };
            if let (Some(lo), Some(hi)) = (min, max)
                && lo > hi
            {
                return false;
            }
            min.is_none_or(|lo| value >= lo) && max.is_none_or(|hi| value <= hi)
        }
        other => double_of(other).is_some_and(|exact| value == exact),
    }
}

/// `HolderSet.contains`: `"#tag"`, `"id"`, or `["id", ...]`.
fn holder_set_matches(env: &PredicateEnv<'_>, registry: &str, set: &Tag, candidate: &str) -> bool {
    let candidate = canonical_id(candidate);
    match set {
        Tag::String(raw) => {
            let text = raw.to_string_lossy();
            if let Some(tag_raw) = text.strip_prefix('#') {
                tag_contains(env, registry, tag_raw, &candidate)
            } else {
                canonical_id(&text) == candidate
            }
        }
        Tag::List(items) => items
            .iter()
            .filter_map(tag_str)
            .any(|id| canonical_id(&id) == candidate),
        Tag::ByteArray(v) if v.is_empty() => false,
        Tag::IntArray(v) if v.is_empty() => false,
        Tag::LongArray(v) if v.is_empty() => false,
        _ => false,
    }
}

fn tag_contains(env: &PredicateEnv<'_>, registry: &str, tag: &str, candidate: &str) -> bool {
    let reg_path = registry.strip_prefix("minecraft:").unwrap_or(registry);
    let cand_short = candidate.strip_prefix("minecraft:").unwrap_or(candidate);
    if reg_path == "item" {
        let short_tag = tag.strip_prefix("minecraft:").unwrap_or(tag);
        let full_tag = canonical_id(tag);
        if env
            .items
            .item_tags
            .get(short_tag)
            .or_else(|| env.items.item_tags.get(&full_tag))
            .is_some_and(|set| set.contains(candidate) || set.contains(cand_short))
        {
            return true;
        }
    }
    (env.tag_lookup)(reg_path, tag)
        .iter()
        .any(|entry| canonical_id(entry) == candidate)
}

/// `CollectionPredicate.test`: `contains`, `count`, and `size`.
fn matches_collection<T>(
    pred: &Tag,
    elements: &[T],
    test_elem: impl Fn(&Tag, &T) -> bool,
) -> bool {
    let Tag::Compound(c) = pred else {
        return false;
    };
    if let Some(contains) = c.get(&"contains".into()) {
        let Some(tests) = list_slice(contains) else {
            return false;
        };
        // `CollectionContentsPredicate.Multiple`: every predicate must match at
        // least one element (a single element may satisfy multiple predicates).
        if !tests
            .iter()
            .all(|p| elements.iter().any(|elem| test_elem(p, elem)))
        {
            return false;
        }
    }
    if let Some(counts) = c.get(&"count".into()) {
        let Some(entries) = list_slice(counts) else {
            return false;
        };
        for entry in entries {
            let Tag::Compound(ec) = entry else {
                return false;
            };
            let (Some(test_p), Some(bound)) = (ec.get(&"test".into()), ec.get(&"count".into()))
            else {
                return false;
            };
            let matched = elements.iter().filter(|e| test_elem(test_p, e)).count() as i64;
            if !matches_int_bounds(matched, bound) {
                return false;
            }
        }
    }
    if let Some(size) = c.get(&"size".into())
        && !matches_int_bounds(elements.len() as i64, size)
    {
        return false;
    }
    true
}

fn list_slice(tag: &Tag) -> Option<&[Tag]> {
    match tag {
        Tag::List(items) => Some(items.as_slice()),
        Tag::ByteArray(v) if v.is_empty() => Some(&[]),
        Tag::IntArray(v) if v.is_empty() => Some(&[]),
        Tag::LongArray(v) if v.is_empty() => Some(&[]),
        _ => None,
    }
}

fn parse_snbt_arg(raw: &str) -> Option<Tag> {
    parse_fully(raw)
        .ok()
        .or_else(|| horizon_core::nbt::parse_snbt(raw).ok().map(|(t, _)| t))
}

/// Evaluate `ItemPredicateArgument` on a live `ItemStack`.
pub(super) fn stack_matches(game: &Game<'_>, predicate: &ItemInput, stack: &ItemStack) -> bool {
    let lookup = |reg: &str, tag: &str| game.tag_entries(reg, tag);
    let env = PredicateEnv::from_game(game, &lookup);
    stack_matches_in(&env, predicate, stack)
}

pub(super) fn stack_component_present(game: &Game<'_>, stack: &ItemStack, id: &str) -> bool {
    let lookup = |reg: &str, tag: &str| game.tag_entries(reg, tag);
    let env = PredicateEnv::from_game(game, &lookup);
    component_present_in(&env, stack, id)
}

fn stack_matches_in(env: &PredicateEnv<'_>, predicate: &ItemInput, stack: &ItemStack) -> bool {
    if !env.item_matches(predicate, &stack.item) {
        return false;
    }
    let Some(tests) = predicate.components.as_deref() else {
        return true;
    };
    let inner = tests
        .trim()
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .unwrap_or("");
    let components = effective_components(stack, env.items.component_defaults.get(&*stack.item));
    let test = |term: &str| -> bool {
        let term = term.trim();
        let (negated, term) = match term.strip_prefix('!') {
            Some(t) => (true, t.trim()),
            None => (false, term),
        };
        let split = term.find(['=', '~']).unwrap_or(term.len());
        let id = canonical_id(term[..split].trim());
        let result = match term[split..].chars().next() {
            Some('=') => {
                let raw = term[split + 1..].trim();
                let Some(want) = parse_snbt_arg(raw) else {
                    return false;
                };
                if id == "minecraft:count" {
                    super::item_count::matches(stack.count, &want)
                } else {
                    match effective_value(env, stack, &components, &id) {
                        Some(have) => exact_component_equals(env, &id, &have, &want, 0),
                        None => false,
                    }
                }
            }
            Some('~') => {
                let raw = term[split + 1..].trim();
                let Some(pred) = parse_snbt_arg(raw) else {
                    return false;
                };
                eval_sub_predicate(env, stack, &components, &id, &pred, 0)
            }
            // `ItemPredicateArgument.PSEUDO_COMPONENTS`: `count` presenceChecker
            // is always true for any stack (`lambda$static$6 -> true`).
            _ if id == "minecraft:count" => true,
            _ => component_present_in(env, stack, &id),
        };
        result != negated
    };
    split_top_level(inner, ',')
        .iter()
        .filter(|t| !t.trim().is_empty())
        .all(|term| split_top_level(term, '|').iter().any(|alt| test(alt)))
}

fn component_present_in(env: &PredicateEnv<'_>, stack: &ItemStack, id: &str) -> bool {
    if let Some(patch) = components_tag(stack.damage, stack.data()) {
        if patch.get(&format!("!{id}")).is_some() {
            return false;
        }
        if patch.get(id).is_some() {
            return true;
        }
    }
    if env
        .items
        .component_defaults
        .get(&*stack.item)
        .is_some_and(|proto| proto.contains_key(&JavaString::from(id)))
    {
        return true;
    }
    env.items
        .get(&stack.item)
        .is_some_and(|item| item.components.iter().any(|key| key == id))
}

fn effective_value(
    env: &PredicateEnv<'_>,
    stack: &ItemStack,
    components: &Compound,
    id: &str,
) -> Option<Tag> {
    if let Some(v) = components.get(&JavaString::from(id)) {
        return Some(v.clone());
    }
    // Minimal test harnesses without `component_defaults` may still know
    // `minecraft:damage` via `item.components`.
    if id == "minecraft:damage" && component_present_in(env, stack, id) {
        return Some(Tag::Int(stack.damage as i32));
    }
    None
}

// ---- Sub-predicates (`~`, vanilla 26.3 DataComponentPredicates) -----------

fn eval_sub_predicate(
    env: &PredicateEnv<'_>,
    stack: &ItemStack,
    components: &Compound,
    id: &str,
    pred: &Tag,
    depth: usize,
) -> bool {
    if depth > MAX_NESTING_DEPTH {
        return false;
    }
    match id {
        "minecraft:count" => super::item_count::matches(stack.count, pred),
        // `CustomDataPredicate` -> `NbtPredicate.matches(DataComponentGetter)`:
        // `getter.getOrDefault(DataComponents.CUSTOM_DATA, CustomData.EMPTY).matchedBy(tag)`.
        "minecraft:custom_data" => {
            let pattern = match pred {
                Tag::Compound(_) => pred.clone(),
                Tag::String(s) => match parse_fully(&s.to_string_lossy()) {
                    Ok(c @ Tag::Compound(_)) => c,
                    _ => return false,
                },
                _ => return false,
            };
            let actual = match components.get(&"minecraft:custom_data".into()) {
                None => Tag::Compound(Compound::new()),
                Some(c @ Tag::Compound(_)) => c.clone(),
                Some(Tag::String(s)) => match parse_fully(&s.to_string_lossy()) {
                    Ok(c @ Tag::Compound(_)) => c,
                    _ => return false,
                },
                Some(_) => return false,
            };
            compare_nbt(&pattern, Some(&actual))
        }
        // `DamagePredicate.matches(DataComponentGetter)`:
        // requires `DAMAGE` present; `maxDamage = getOrDefault(MAX_DAMAGE, 0)`;
        // checks `durability.matches(maxDamage - damage)` and `damage.matches(damage)`.
        "minecraft:damage" => {
            let Tag::Compound(c) = pred else {
                return false;
            };
            let Some(damage_tag) = effective_value(env, stack, components, "minecraft:damage")
            else {
                return false;
            };
            let Some(damage) = int_of(&damage_tag) else {
                return false;
            };
            let max_damage = match components.get(&"minecraft:max_damage".into()) {
                Some(v) => int_of(v).unwrap_or(0),
                None => {
                    if component_present_in(env, stack, "minecraft:max_damage") {
                        env.items
                            .get(&stack.item)
                            .map_or(0, |item| i64::from(item.max_damage))
                    } else {
                        0
                    }
                }
            };
            let durability = max_damage - damage;
            c.get(&"durability".into())
                .is_none_or(|b| matches_int_bounds(durability, b))
                && c.get(&"damage".into())
                    .is_none_or(|b| matches_int_bounds(damage, b))
        }
        // `EnchantmentsPredicate$Enchantments` / `StoredEnchantments`:
        // `SingleComponentItemPredicate` (component must exist), then every
        // `EnchantmentPredicate` in the list must be `containedIn(enchantments)`.
        "minecraft:enchantments" | "minecraft:stored_enchantments" => {
            let Some(have) = effective_value(env, stack, components, id) else {
                return false;
            };
            let Some(entries) = list_slice(pred) else {
                return false;
            };
            let levels = extract_enchantment_levels(&have);
            entries
                .iter()
                .all(|ep| matches_enchantment_predicate(env, ep, &levels))
        }
        // `PotionsPredicate` (26.3): `{potions?: HolderSet<Potion>, effects?: CollectionPredicate}`.
        "minecraft:potion_contents" => {
            let Some(have) = effective_value(env, stack, components, id) else {
                return false;
            };
            matches_potions_predicate(env, pred, &have)
        }
        // `ContainerPredicate`: `{items?: CollectionPredicate<ItemPredicate>}` over nonEmptyItems.
        "minecraft:container" => {
            let Some(have) = effective_value(env, stack, components, id) else {
                return false;
            };
            let Tag::Compound(c) = pred else {
                return false;
            };
            let Some(items_pred) = c.get(&"items".into()) else {
                return true;
            };
            let items = extract_container_items(&have);
            matches_collection(items_pred, &items, |ip, item_stack| {
                matches_item_predicate_nbt(env, ip, item_stack, depth + 1)
            })
        }
        // `BundlePredicate`: `{items?: CollectionPredicate<ItemPredicate>}` over bundle items.
        "minecraft:bundle_contents" => {
            let Some(have) = effective_value(env, stack, components, id) else {
                return false;
            };
            let Tag::Compound(c) = pred else {
                return false;
            };
            let Some(items_pred) = c.get(&"items".into()) else {
                return true;
            };
            let items = extract_bundle_items(&have);
            matches_collection(items_pred, &items, |ip, item_stack| {
                matches_item_predicate_nbt(env, ip, item_stack, depth + 1)
            })
        }
        // `FireworkExplosionPredicate`: `{shape?, has_twinkle?, has_trail?}`.
        "minecraft:firework_explosion" => {
            let Some(have) = effective_value(env, stack, components, id) else {
                return false;
            };
            matches_firework_explosion_predicate(pred, &have)
        }
        // `FireworksPredicate`: `{explosions?: CollectionPredicate, flight_duration?: Ints}`.
        "minecraft:fireworks" => {
            let Some(have) = effective_value(env, stack, components, id) else {
                return false;
            };
            let (Tag::Compound(pc), Tag::Compound(hc)) = (pred, &have) else {
                return false;
            };
            let flight = match hc.get(&"flight_duration".into()) {
                Some(v) => match unsigned_byte_of(v) {
                    Some(n) => i64::from(n),
                    None => return false,
                },
                None => 0,
            };
            let explosions = hc
                .get(&"explosions".into())
                .and_then(list_slice)
                .unwrap_or(&[]);
            pc.get(&"explosions".into()).is_none_or(|ep| {
                matches_collection(ep, explosions, matches_firework_explosion_predicate)
            }) && pc
                .get(&"flight_duration".into())
                .is_none_or(|b| matches_int_bounds(flight, b))
        }
        // `WritableBookPredicate`: `{pages?: CollectionPredicate<String>}` against raw page text.
        "minecraft:writable_book_content" => {
            let Some(have) = effective_value(env, stack, components, id) else {
                return false;
            };
            let (Tag::Compound(pc), Tag::Compound(hc)) = (pred, &have) else {
                return false;
            };
            let Some(pages_pred) = pc.get(&"pages".into()) else {
                return true;
            };
            let pages: Vec<String> = hc
                .get(&"pages".into())
                .and_then(list_slice)
                .unwrap_or(&[])
                .iter()
                .filter_map(filterable_raw_str)
                .collect();
            matches_collection(pages_pred, &pages, |want, raw| {
                tag_str(want).is_some_and(|w| w == *raw)
            })
        }
        // `WrittenBookPredicate`: `{pages?, author?, title?, generation?, resolved?}`.
        "minecraft:written_book_content" => {
            let Some(have) = effective_value(env, stack, components, id) else {
                return false;
            };
            let (Tag::Compound(pc), Tag::Compound(hc)) = (pred, &have) else {
                return false;
            };
            if let Some(want_author) = pc.get(&"author".into()) {
                let have_author = hc.get(&"author".into()).and_then(tag_str);
                if tag_str(want_author) != have_author {
                    return false;
                }
            }
            if let Some(want_title) = pc.get(&"title".into()) {
                let have_title = hc.get(&"title".into()).and_then(filterable_raw_str);
                if tag_str(want_title) != have_title {
                    return false;
                }
            }
            let generation = hc.get(&"generation".into()).and_then(int_of).unwrap_or(0);
            if pc
                .get(&"generation".into())
                .is_some_and(|b| !matches_int_bounds(generation, b))
            {
                return false;
            }
            if let Some(want_resolved) = pc.get(&"resolved".into()) {
                let have_resolved = hc
                    .get(&"resolved".into())
                    .and_then(component_bool)
                    .unwrap_or(false);
                if component_bool(want_resolved) != Some(have_resolved) {
                    return false;
                }
            }
            if let Some(pages_pred) = pc.get(&"pages".into()) {
                let pages: Vec<Tag> = hc
                    .get(&"pages".into())
                    .and_then(list_slice)
                    .unwrap_or(&[])
                    .iter()
                    .map(filterable_raw_tag)
                    .collect();
                if !matches_collection(pages_pred, &pages, text_component_equal) {
                    return false;
                }
            }
            true
        }
        // `AttributeModifiersPredicate`: `{modifiers?: CollectionPredicate<EntryPredicate>}`.
        "minecraft:attribute_modifiers" => {
            let Some(have) = effective_value(env, stack, components, id) else {
                return false;
            };
            let Tag::Compound(pc) = pred else {
                return false;
            };
            let Some(mods_pred) = pc.get(&"modifiers".into()) else {
                return true;
            };
            let entries = extract_attribute_modifiers(&have);
            matches_collection(mods_pred, &entries, |ep, entry| {
                matches_attribute_entry_predicate(env, ep, entry)
            })
        }
        // `TrimPredicate`: `{material?: HolderSet<TrimMaterial>, pattern?: HolderSet<TrimPattern>}`.
        "minecraft:trim" => {
            let Some(have) = effective_value(env, stack, components, id) else {
                return false;
            };
            let (Tag::Compound(pc), Tag::Compound(hc)) = (pred, &have) else {
                return false;
            };
            if let Some(mat_set) = pc.get(&"material".into()) {
                let Some(mat) = hc.get(&"material".into()).and_then(tag_str) else {
                    return false;
                };
                if !holder_set_matches(env, "minecraft:trim_material", mat_set, &mat) {
                    return false;
                }
            }
            if let Some(pat_set) = pc.get(&"pattern".into()) {
                let Some(pat) = hc.get(&"pattern".into()).and_then(tag_str) else {
                    return false;
                };
                if !holder_set_matches(env, "minecraft:trim_pattern", pat_set, &pat) {
                    return false;
                }
            }
            true
        }
        // `JukeboxPlayablePredicate`: `{song?: HolderSet<JukeboxSong>}`.
        "minecraft:jukebox_playable" => {
            let Some(have) = effective_value(env, stack, components, id) else {
                return false;
            };
            let Tag::Compound(pc) = pred else {
                return false;
            };
            let Some(song_set) = pc.get(&"song".into()) else {
                return true;
            };
            let song = match &have {
                Tag::String(s) => Some(s.to_string_lossy()),
                Tag::Compound(c) => c.get(&"song".into()).and_then(tag_str),
                _ => None,
            };
            song.is_some_and(|s| holder_set_matches(env, "minecraft:jukebox_song", song_set, &s))
        }
        // `VillagerTypePredicate`: `HolderSet<VillagerType>` directly.
        "minecraft:villager/variant" => {
            let Some(have) = effective_value(env, stack, components, id) else {
                return false;
            };
            tag_str(&have)
                .is_some_and(|v| holder_set_matches(env, "minecraft:villager_type", pred, &v))
        }
        // `ItemPredicateArgument.createComponentExistencePredicate`:
        // any non-predicate `DataComponentType` used with `~{}` checks `stack.has(type)`.
        _ => matches!(pred, Tag::Compound(_)) && component_present_in(env, stack, id),
    }
}

fn extract_enchantment_levels(tag: &Tag) -> Vec<(String, i64)> {
    let Tag::Compound(map) = tag else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(k, v)| {
            let lvl = int_of(v)?;
            (lvl > 0).then(|| (canonical_id(&k.to_string_lossy()), lvl))
        })
        .collect()
}

/// `EnchantmentPredicate.containedIn(ItemEnchantments)`.
fn matches_enchantment_predicate(
    env: &PredicateEnv<'_>,
    pred: &Tag,
    levels: &[(String, i64)],
) -> bool {
    let Tag::Compound(c) = pred else {
        return false;
    };
    let level_bound = c.get(&"levels".into());
    if let Some(ench_set) = c.get(&"enchantments".into()) {
        return levels.iter().any(|(id, lvl)| {
            *lvl != 0
                && holder_set_matches(env, "minecraft:enchantment", ench_set, id)
                && level_bound.is_none_or(|b| matches_int_bounds(*lvl, b))
        });
    }
    if let Some(bound) = level_bound
        && !is_any_int_bounds(bound)
    {
        return levels
            .iter()
            .any(|(_, lvl)| matches_int_bounds(*lvl, bound));
    }
    !levels.is_empty()
}

struct EffectInstance {
    id: String,
    amplifier: i64,
    duration: i64,
    ambient: bool,
    visible: bool,
}

fn extract_potion_contents(have: &Tag) -> (Option<String>, Vec<EffectInstance>) {
    let (potion_id, custom_list) = match have {
        Tag::String(s) => (Some(canonical_id(&s.to_string_lossy())), None),
        Tag::Compound(c) => (
            c.get(&"potion".into())
                .and_then(tag_str)
                .map(|s| canonical_id(&s)),
            c.get(&"custom_effects".into()).and_then(list_slice),
        ),
        _ => (None, None),
    };
    let mut effects = Vec::new();
    if let Some(pid) = potion_id.as_deref()
        && let Some(def) = horizon_client_core::potions::potion(pid)
    {
        for (eid, dur, amp) in def.effects {
            effects.push(EffectInstance {
                id: canonical_id(eid),
                amplifier: i64::from(*amp),
                duration: i64::from(*dur),
                ambient: false,
                visible: true,
            });
        }
    }
    for entry in custom_list.unwrap_or(&[]) {
        let Tag::Compound(ec) = entry else {
            continue;
        };
        let Some(eid) = ec.get(&"id".into()).and_then(tag_str) else {
            continue;
        };
        effects.push(EffectInstance {
            id: canonical_id(&eid),
            amplifier: ec
                .get(&"amplifier".into())
                .and_then(unsigned_byte_of)
                .map_or(0, i64::from),
            duration: ec.get(&"duration".into()).and_then(int_of).unwrap_or(0),
            ambient: ec
                .get(&"ambient".into())
                .and_then(component_bool)
                .unwrap_or(false),
            visible: ec
                .get(&"show_particles".into())
                .and_then(component_bool)
                .unwrap_or(true),
        });
    }
    (potion_id, effects)
}

fn matches_potions_predicate(env: &PredicateEnv<'_>, pred: &Tag, have: &Tag) -> bool {
    let Tag::Compound(pc) = pred else {
        return false;
    };
    let (potion_id, effects) = extract_potion_contents(have);
    if let Some(potions_set) = pc.get(&"potions".into()) {
        let Some(pid) = potion_id.as_deref() else {
            return false;
        };
        if !holder_set_matches(env, "minecraft:potion", potions_set, pid) {
            return false;
        }
    }
    if let Some(effects_pred) = pc.get(&"effects".into()) {
        return matches_collection(effects_pred, &effects, matches_mob_effects_predicate);
    }
    true
}

/// `MobEffectsPredicate.test(MobEffectInstance)`: looks up `inst.getEffect()` in `effectMap`.
fn matches_mob_effects_predicate(pred: &Tag, inst: &EffectInstance) -> bool {
    let Tag::Compound(map) = pred else {
        return false;
    };
    let Some((_, entry)) = map
        .iter()
        .find(|(k, _)| canonical_id(&k.to_string_lossy()) == inst.id)
    else {
        return false;
    };
    let Tag::Compound(ec) = entry else {
        return false;
    };
    ec.get(&"amplifier".into())
        .is_none_or(|b| matches_int_bounds(inst.amplifier, b))
        && ec
            .get(&"duration".into())
            .is_none_or(|b| matches_int_bounds(inst.duration, b))
        && ec
            .get(&"ambient".into())
            .is_none_or(|b| component_bool(b) == Some(inst.ambient))
        && ec
            .get(&"visible".into())
            .is_none_or(|b| component_bool(b) == Some(inst.visible))
}

fn extract_container_items(have: &Tag) -> Vec<ItemStack> {
    list_slice(have)
        .unwrap_or(&[])
        .iter()
        .filter_map(|entry| {
            let item_tag = entry.get("item")?;
            item_from_nbt(item_tag)
        })
        .filter(|stack| stack.count > 0 && stack.item.as_ref() != "minecraft:air")
        .collect()
}

fn extract_bundle_items(have: &Tag) -> Vec<ItemStack> {
    list_slice(have)
        .unwrap_or(&[])
        .iter()
        .filter_map(item_from_nbt)
        .filter(|stack| stack.count > 0 && stack.item.as_ref() != "minecraft:air")
        .collect()
}

/// `ItemPredicate.test(ItemInstance)` as encoded in NBT (`ContainerPredicate`, `BundlePredicate`).
fn matches_item_predicate_nbt(
    env: &PredicateEnv<'_>,
    pred: &Tag,
    stack: &ItemStack,
    depth: usize,
) -> bool {
    if depth > MAX_NESTING_DEPTH {
        return false;
    }
    let Tag::Compound(c) = pred else {
        return false;
    };
    if let Some(items_set) = c.get(&"items".into())
        && !holder_set_matches(env, "minecraft:item", items_set, &stack.item)
    {
        return false;
    }
    if let Some(count_bound) = c.get(&"count".into())
        && !matches_int_bounds(i64::from(stack.count), count_bound)
    {
        return false;
    }
    let need_components =
        c.get(&"components".into()).is_some() || c.get(&"predicates".into()).is_some();
    if !need_components {
        return true;
    }
    let components = effective_components(stack, env.items.component_defaults.get(&*stack.item));
    if let Some(exact) = c.get(&"components".into()) {
        let Tag::Compound(map) = exact else {
            return false;
        };
        for (k, want) in map {
            let id = canonical_id(&k.to_string_lossy());
            let Some(have) = effective_value(env, stack, &components, &id) else {
                return false;
            };
            if !exact_component_equals(env, &id, &have, want, depth + 1) {
                return false;
            }
        }
    }
    if let Some(partial) = c.get(&"predicates".into()) {
        let Tag::Compound(map) = partial else {
            return false;
        };
        for (k, sub_pred) in map {
            let id = canonical_id(&k.to_string_lossy());
            if !eval_sub_predicate(env, stack, &components, &id, sub_pred, depth + 1) {
                return false;
            }
        }
    }
    true
}

fn matches_firework_explosion_predicate(pred: &Tag, have: &Tag) -> bool {
    let (Tag::Compound(pc), Tag::Compound(hc)) = (pred, have) else {
        return false;
    };
    if let Some(shape) = pc.get(&"shape".into())
        && tag_str(shape) != hc.get(&"shape".into()).and_then(tag_str)
    {
        return false;
    }
    if let Some(twinkle) = pc.get(&"has_twinkle".into()) {
        let actual = hc
            .get(&"has_twinkle".into())
            .and_then(component_bool)
            .unwrap_or(false);
        if component_bool(twinkle) != Some(actual) {
            return false;
        }
    }
    if let Some(trail) = pc.get(&"has_trail".into()) {
        let actual = hc
            .get(&"has_trail".into())
            .and_then(component_bool)
            .unwrap_or(false);
        if component_bool(trail) != Some(actual) {
            return false;
        }
    }
    true
}

fn filterable_raw_str(tag: &Tag) -> Option<String> {
    match tag {
        Tag::String(s) => Some(s.to_string_lossy()),
        Tag::Compound(c) => c.get(&"raw".into()).and_then(tag_str),
        _ => None,
    }
}

fn filterable_raw_tag(tag: &Tag) -> Tag {
    match tag {
        Tag::Compound(c) => c.get(&"raw".into()).cloned().unwrap_or_else(|| tag.clone()),
        _ => tag.clone(),
    }
}

struct AttributeEntry {
    attribute: String,
    id: String,
    amount: f64,
    operation: String,
    slot: String,
}

fn extract_attribute_modifiers(have: &Tag) -> Vec<AttributeEntry> {
    let list = match have {
        Tag::Compound(c) => c.get(&"modifiers".into()).and_then(list_slice),
        _ => list_slice(have),
    };
    list.unwrap_or(&[])
        .iter()
        .filter_map(|entry| {
            let Tag::Compound(ec) = entry else {
                return None;
            };
            let attribute = canonical_id(&ec.get(&"type".into()).and_then(tag_str)?);
            let id = canonical_id(&ec.get(&"id".into()).and_then(tag_str)?);
            let amount = double_of(ec.get(&"amount".into())?)?;
            let operation = ec.get(&"operation".into()).and_then(tag_str)?;
            let slot = ec
                .get(&"slot".into())
                .and_then(tag_str)
                .unwrap_or_else(|| "any".to_owned());
            Some(AttributeEntry {
                attribute,
                id,
                amount,
                operation,
                slot,
            })
        })
        .collect()
}

fn matches_attribute_entry_predicate(
    env: &PredicateEnv<'_>,
    pred: &Tag,
    entry: &AttributeEntry,
) -> bool {
    let Tag::Compound(c) = pred else {
        return false;
    };
    if let Some(attr) = c.get(&"attribute".into())
        && !holder_set_matches(env, "minecraft:attribute", attr, &entry.attribute)
    {
        return false;
    }
    if let Some(id) = c.get(&"id".into())
        && tag_str(id).map(|s| canonical_id(&s)).as_deref() != Some(entry.id.as_str())
    {
        return false;
    }
    if let Some(amount) = c.get(&"amount".into())
        && !matches_double_bounds(entry.amount, amount)
    {
        return false;
    }
    if let Some(op) = c.get(&"operation".into())
        && tag_str(op).as_deref() != Some(entry.operation.as_str())
    {
        return false;
    }
    if let Some(slot) = c.get(&"slot".into())
        && tag_str(slot).as_deref() != Some(entry.slot.as_str())
    {
        return false;
    }
    true
}

// ---- Exact component equality (`=`, vanilla Objects.equals on decoded values) ----

fn exact_component_equals(
    env: &PredicateEnv<'_>,
    id: &str,
    have: &Tag,
    want: &Tag,
    depth: usize,
) -> bool {
    if depth > MAX_NESTING_DEPTH {
        return false;
    }
    match id {
        // Raw NBT components: `CustomData.equals` compares `CompoundTag.equals`.
        "minecraft:custom_data" | "minecraft:bucket_entity_data" => {
            let norm = |tag: &Tag| match tag {
                Tag::Compound(_) => Some(tag.clone()),
                Tag::String(s) => match parse_fully(&s.to_string_lossy()) {
                    Ok(c @ Tag::Compound(_)) => Some(c),
                    _ => None,
                },
                _ => None,
            };
            matches!((norm(have), norm(want)), (Some(a), Some(b)) if a == b)
        }
        "minecraft:entity_data" | "minecraft:block_entity_data" => {
            typed_entity_data_equal(have, want)
        }
        "minecraft:enchantments" | "minecraft:stored_enchantments" => {
            let norm = |tag: &Tag| -> Option<std::collections::BTreeMap<String, i32>> {
                let Tag::Compound(c) = tag else {
                    return None;
                };
                c.iter()
                    .map(|(k, v)| Some((canonical_id(&k.to_string_lossy()), int_of(v)? as i32)))
                    .collect()
            };
            matches!((norm(have), norm(want)), (Some(a), Some(b)) if a == b)
        }
        "minecraft:custom_name" | "minecraft:item_name" => text_component_equal(have, want),
        "minecraft:lore" => {
            let (Some(a), Some(b)) = (list_slice(have), list_slice(want)) else {
                return false;
            };
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|(x, y)| text_component_equal(x, y))
        }
        "minecraft:fireworks" => {
            matches!((norm_fireworks(have), norm_fireworks(want)), (Some(a), Some(b)) if a == b)
        }
        "minecraft:firework_explosion" => {
            matches!((norm_firework_explosion(have), norm_firework_explosion(want)), (Some(a), Some(b)) if a == b)
        }
        "minecraft:writable_book_content" => {
            matches!((norm_writable_book(have), norm_writable_book(want)), (Some(a), Some(b)) if a == b)
        }
        "minecraft:written_book_content" => written_book_equal(have, want),
        "minecraft:attribute_modifiers" => {
            matches!((norm_attribute_modifiers(have), norm_attribute_modifiers(want)), (Some(a), Some(b)) if a == b)
        }
        "minecraft:death_protection" => death_protection_equal(have, want),
        "minecraft:consumable" => consumable_equal(have, want),
        "minecraft:equippable" => equippable_equal(have, want),
        "minecraft:tool" => tool_equal(have, want),
        "minecraft:potion_contents" => potion_contents_equal(have, want),
        "minecraft:suspicious_stew_effects" => {
            matches!((norm_stew_effects(have), norm_stew_effects(want)), (Some(a), Some(b)) if a == b)
        }
        "minecraft:banner_patterns" => {
            matches!((norm_banner_patterns(have), norm_banner_patterns(want)), (Some(a), Some(b)) if a == b)
        }
        "minecraft:pot_decorations" => pot_decorations_equal(env, have, want, depth + 1),
        "minecraft:trim" => trim_equal(have, want),
        "minecraft:break_sound" => sound_event_equal(have, want),
        "minecraft:jukebox_playable" => {
            matches!((tag_str(have), tag_str(want)), (Some(a), Some(b)) if canonical_id(&a) == canonical_id(&b))
        }
        "minecraft:instrument" => instrument_equal(have, want),
        "minecraft:map_color" => matches!((int_of(have), int_of(want)), (Some(a), Some(b)) if a == b),
        "minecraft:interact_animation" => {
            matches!((SwingAnimation::decode(have), SwingAnimation::decode(want)), (Some(a), Some(b)) if a == b)
        }
        "minecraft:repairable" => {
            matches!(
                (repairable_items(have), repairable_items(want)),
                (Some(a), Some(b)) if holder_set_equal(a, b)
            )
        }
        "minecraft:use_remainder" => item_template_equal(env, have, want, depth + 1),
        "minecraft:container" => container_equal(env, have, want, depth + 1),
        "minecraft:bundle_contents" | "minecraft:charged_projectiles" => {
            let (Some(a), Some(b)) = (list_slice(have), list_slice(want)) else {
                return false;
            };
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|(x, y)| item_template_equal(env, x, y, depth + 1))
        }
        "minecraft:bees" => bees_equal(have, want),
        _ => {
            // Shared decoded component codecs (covers all `DECODED_COMPONENTS`:
            // food, weapon, use_effects, use_cooldown, tooltip_display,
            // lodestone_tracker, container_loot, custom_model_data, damage,
            // max_damage, max_stack_size, repair_cost, dyed_color, etc.).
            if DECODED_COMPONENTS.contains(&id)
                && let (Ok(a), Ok(b)) = (
                    decoded_component_value(id, have, env.registries),
                    decoded_component_value(id, want, env.registries),
                )
            {
                if a == b {
                    return true;
                }
                let mk = |v: Tag| {
                    let mut c = Compound::new();
                    c.insert(JavaString::from(id), v);
                    let (damage, extra) = decode_components(Some(&Tag::Compound(c)));
                    ItemStack {
                        item: "minecraft:air".into(),
                        count: 1,
                        damage,
                        extra,
                    }
                };
                return component_equal(id, &mk(a), &mk(b));
            }
            (compare_nbt(want, Some(have)) && compare_nbt(have, Some(want)))
                || same_component_value(id, have, want)
        }
    }
}

fn repairable_items(tag: &Tag) -> Option<&Tag> {
    match tag {
        Tag::Compound(c) => c.get(&"items".into()),
        _ => None,
    }
}

fn parse_text_color(raw: &str) -> Option<u32> {
    if let Some(hex) = raw.strip_prefix('#') {
        let digits = hex.strip_prefix(['+', '-']).unwrap_or(hex);
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let value = i64::from_str_radix(hex, 16).ok()?;
        (0..=0xFF_FFFF).contains(&value).then_some(value as u32)
    } else {
        horizon_client_core::text::named_color(raw)
    }
}

fn parse_shadow_color(tag: &Tag) -> Option<u32> {
    if let Some(packed) = int_of(tag) {
        return i32::try_from(packed).ok().map(|n| n as u32);
    }
    let items = list_slice(tag)?;
    if items.len() != 4 {
        return None;
    }
    let mut rgba = [0u32; 4];
    for (slot, item) in rgba.iter_mut().zip(items) {
        let f = float_of(item)?;
        if !f.is_finite() {
            return None;
        }
        *slot = (f.clamp(0.0, 1.0) * 255.0).floor() as u32;
    }
    Some((rgba[3] << 24) | (rgba[0] << 16) | (rgba[1] << 8) | rgba[2])
}

#[derive(PartialEq, Eq)]
enum NormArg {
    String(String),
    Component(NormComponent),
    Primitive(Tag),
}

#[derive(PartialEq, Eq)]
enum NormContents {
    Plain(String),
    Translatable {
        key: String,
        fallback: Option<String>,
        args: Vec<NormArg>,
    },
    Keybind(String),
    Score {
        name: String,
        objective: String,
    },
    Selector {
        pattern: String,
        separator: Option<Box<NormComponent>>,
    },
    Nbt {
        path: String,
        interpret: bool,
        separator: Option<Box<NormComponent>>,
        source_kind: &'static str,
        source: String,
    },
    Object {
        kind: &'static str,
        primary: Tag,
        secondary: Option<String>,
        hat: bool,
        fallback: Option<Box<NormComponent>>,
    },
}

#[derive(PartialEq, Eq)]
enum NormHover {
    ShowText(Box<NormComponent>),
    Other(Tag),
}

#[derive(Default, PartialEq, Eq)]
struct NormStyle {
    color: Option<u32>,
    shadow_color: Option<u32>,
    bold: Option<bool>,
    italic: Option<bool>,
    underlined: Option<bool>,
    strikethrough: Option<bool>,
    obfuscated: Option<bool>,
    font: Option<String>,
    insertion: Option<String>,
    click_event: Option<Tag>,
    hover_event: Option<NormHover>,
}

#[derive(PartialEq, Eq)]
struct NormComponent {
    contents: NormContents,
    siblings: Vec<NormComponent>,
    style: NormStyle,
}

fn norm_click_event(tag: &Tag) -> Option<Tag> {
    let Tag::Compound(c) = tag else {
        return None;
    };
    let action = tag_str(c.get(&"action".into())?)?;
    let mut out = Compound::new();
    out.insert("action".into(), Tag::String(action.as_str().into()));
    match action.as_str() {
        "open_url" => {
            let url = tag_str(c.get(&"url".into())?)?;
            out.insert("url".into(), Tag::String(url.as_str().into()));
        }
        "open_file" => {
            let path = tag_str(c.get(&"path".into())?)?;
            out.insert("path".into(), Tag::String(path.as_str().into()));
        }
        "run_command" | "suggest_command" => {
            let cmd = tag_str(c.get(&"command".into())?)?;
            out.insert("command".into(), Tag::String(cmd.as_str().into()));
        }
        "change_page" => {
            let page = int_of(c.get(&"page".into())?)? as i32;
            out.insert("page".into(), Tag::Int(page));
        }
        "copy_to_clipboard" => {
            let value = tag_str(c.get(&"value".into())?)?;
            out.insert("value".into(), Tag::String(value.as_str().into()));
        }
        "show_dialog" => {
            let dialog = c.get(&"dialog".into())?;
            out.insert("dialog".into(), dialog.clone());
        }
        "custom" => {
            let id = canonical_id(&tag_str(c.get(&"id".into())?)?);
            out.insert("id".into(), Tag::String(id.as_str().into()));
            if let Some(payload) = c.get(&"payload".into()) {
                out.insert("payload".into(), payload.clone());
            }
        }
        _ => return None,
    }
    Some(Tag::Compound(out))
}

fn norm_hover_event(tag: &Tag, depth: usize) -> Option<NormHover> {
    let Tag::Compound(c) = tag else {
        return None;
    };
    let action = tag_str(c.get(&"action".into())?)?;
    match action.as_str() {
        "show_text" => {
            let value = norm_text_component(c.get(&"value".into())?, depth + 1)?;
            Some(NormHover::ShowText(Box::new(value)))
        }
        "show_item" => {
            let mut out = c.clone();
            let id = canonical_id(&tag_str(c.get(&"id".into())?)?);
            out.insert("id".into(), Tag::String(id.as_str().into()));
            Some(NormHover::Other(Tag::Compound(out)))
        }
        "show_entity" => {
            let mut out = c.clone();
            let id = canonical_id(&tag_str(c.get(&"id".into())?)?);
            out.insert("id".into(), Tag::String(id.as_str().into()));
            Some(NormHover::Other(Tag::Compound(out)))
        }
        _ => None,
    }
}

fn norm_text_contents(c: &Compound, depth: usize) -> Option<NormContents> {
    let kind = match c.get(&"type".into()) {
        Some(v) => Some(tag_str(v)?),
        None => None,
    };
    let kind_name = match kind.as_deref() {
        Some(
            k @ ("text" | "translatable" | "keybind" | "score" | "selector" | "nbt" | "object"),
        ) => k,
        Some(_) => return None,
        None => {
            if c.contains_key(&"text".into()) {
                "text"
            } else if c.contains_key(&"translate".into()) {
                "translatable"
            } else if c.contains_key(&"keybind".into()) {
                "keybind"
            } else if c.contains_key(&"score".into()) {
                "score"
            } else if c.contains_key(&"selector".into()) {
                "selector"
            } else if c.contains_key(&"nbt".into()) {
                "nbt"
            } else if c.contains_key(&"sprite".into())
                || c.contains_key(&"player".into())
                || c.contains_key(&"object".into())
            {
                "object"
            } else {
                return None;
            }
        }
    };
    match kind_name {
        "text" => Some(NormContents::Plain(tag_str(c.get(&"text".into())?)?)),
        "translatable" => {
            let key = tag_str(c.get(&"translate".into())?)?;
            let fallback = match c.get(&"fallback".into()) {
                Some(v) => Some(tag_str(v)?),
                None => None,
            };
            let args = match c.get(&"with".into()) {
                Some(v) => list_slice(v)?
                    .iter()
                    .map(|arg| match arg {
                        Tag::String(s) => Some(NormArg::String(s.to_string_lossy())),
                        Tag::Compound(_) | Tag::List(_) => {
                            Some(NormArg::Component(norm_text_component(arg, depth + 1)?))
                        }
                        other => Some(NormArg::Primitive(other.clone())),
                    })
                    .collect::<Option<Vec<_>>>()?,
                None => Vec::new(),
            };
            Some(NormContents::Translatable {
                key,
                fallback,
                args,
            })
        }
        "keybind" => Some(NormContents::Keybind(tag_str(c.get(&"keybind".into())?)?)),
        "score" => {
            let Tag::Compound(sc) = c.get(&"score".into())? else {
                return None;
            };
            Some(NormContents::Score {
                name: tag_str(sc.get(&"name".into())?)?,
                objective: tag_str(sc.get(&"objective".into())?)?,
            })
        }
        "selector" => {
            let pattern = tag_str(c.get(&"selector".into())?)?;
            let separator = match c.get(&"separator".into()) {
                Some(v) => Some(Box::new(norm_text_component(v, depth + 1)?)),
                None => None,
            };
            Some(NormContents::Selector { pattern, separator })
        }
        "nbt" => {
            let path = tag_str(c.get(&"nbt".into())?)?;
            let interpret = match c.get(&"interpret".into()) {
                Some(v) => component_bool(v)?,
                None => false,
            };
            let separator = match c.get(&"separator".into()) {
                Some(v) => Some(Box::new(norm_text_component(v, depth + 1)?)),
                None => None,
            };
            let (source_kind, source) = if let Some(v) = c.get(&"block".into()) {
                ("block", tag_str(v)?)
            } else if let Some(v) = c.get(&"entity".into()) {
                ("entity", tag_str(v)?)
            } else {
                let v = c.get(&"storage".into())?;
                ("storage", canonical_id(&tag_str(v)?))
            };
            Some(NormContents::Nbt {
                path,
                interpret,
                separator,
                source_kind,
                source,
            })
        }
        "object" => {
            let obj_kind = match c.get(&"object".into()) {
                Some(v) => Some(tag_str(v)?),
                None => None,
            };
            let fallback = match c.get(&"fallback".into()) {
                Some(v) => Some(Box::new(norm_text_component(v, depth + 1)?)),
                None => None,
            };
            match obj_kind.as_deref() {
                Some("atlas") | None if c.contains_key(&"sprite".into()) => {
                    let sprite = canonical_id(&tag_str(c.get(&"sprite".into())?)?);
                    let atlas = match c.get(&"atlas".into()) {
                        Some(v) => canonical_id(&tag_str(v)?),
                        None => "minecraft:blocks".to_owned(),
                    };
                    Some(NormContents::Object {
                        kind: "atlas",
                        primary: Tag::String(sprite.as_str().into()),
                        secondary: Some(atlas),
                        hat: false,
                        fallback,
                    })
                }
                Some("player") | None if c.contains_key(&"player".into()) => {
                    let profile = c.get(&"player".into())?.clone();
                    let hat = match c.get(&"hat".into()) {
                        Some(v) => component_bool(v)?,
                        None => true,
                    };
                    Some(NormContents::Object {
                        kind: "player",
                        primary: profile,
                        secondary: None,
                        hat,
                        fallback,
                    })
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Normalize any `ComponentSerialization.CODEC` NBT form (bare string, non-empty
/// list `[first, sib...]`, or full compound `{...}`) into vanilla's decoded
/// `MutableComponent(contents, siblings, style)` representation.
fn norm_text_component(tag: &Tag, depth: usize) -> Option<NormComponent> {
    if depth > MAX_NESTING_DEPTH {
        return None;
    }
    match tag {
        Tag::String(s) => Some(NormComponent {
            contents: NormContents::Plain(s.to_string_lossy()),
            siblings: Vec::new(),
            style: NormStyle::default(),
        }),
        Tag::List(items) => {
            let (first, rest) = items.split_first()?;
            let mut base = norm_text_component(first, depth + 1)?;
            for sib in rest {
                base.siblings.push(norm_text_component(sib, depth + 1)?);
            }
            Some(base)
        }
        Tag::Compound(c) => {
            let contents = norm_text_contents(c, depth)?;
            let siblings = match c.get(&"extra".into()) {
                Some(v) => list_slice(v)?
                    .iter()
                    .map(|child| norm_text_component(child, depth + 1))
                    .collect::<Option<Vec<_>>>()?,
                None => Vec::new(),
            };
            let color = match c.get(&"color".into()) {
                Some(v) => Some(parse_text_color(&tag_str(v)?)?),
                None => None,
            };
            let shadow_color = match c.get(&"shadow_color".into()) {
                Some(v) => Some(parse_shadow_color(v)?),
                None => None,
            };
            let opt_bool = |key: &str| -> Option<Option<bool>> {
                match c.get(&JavaString::from(key)) {
                    Some(v) => Some(Some(component_bool(v)?)),
                    None => Some(None),
                }
            };
            let font = match c.get(&"font".into()) {
                Some(v) => Some(canonical_id(&tag_str(v)?)),
                None => None,
            };
            let insertion = match c.get(&"insertion".into()) {
                Some(v) => Some(tag_str(v)?),
                None => None,
            };
            let click_event = match c.get(&"click_event".into()) {
                Some(v) => Some(norm_click_event(v)?),
                None => None,
            };
            let hover_event = match c.get(&"hover_event".into()) {
                Some(v) => Some(norm_hover_event(v, depth)?),
                None => None,
            };
            Some(NormComponent {
                contents,
                siblings,
                style: NormStyle {
                    color,
                    shadow_color,
                    bold: opt_bool("bold")?,
                    italic: opt_bool("italic")?,
                    underlined: opt_bool("underlined")?,
                    strikethrough: opt_bool("strikethrough")?,
                    obfuscated: opt_bool("obfuscated")?,
                    font,
                    insertion,
                    click_event,
                    hover_event,
                },
            })
        }
        _ => None,
    }
}

fn text_component_equal(a: &Tag, b: &Tag) -> bool {
    matches!(
        (norm_text_component(a, 0), norm_text_component(b, 0)),
        (Some(x), Some(y)) if x == y
    )
}

#[derive(PartialEq, Eq)]
struct NormSoundEvent {
    id: String,
    range_bits: Option<u32>,
}

fn norm_sound_event(tag: &Tag) -> Option<NormSoundEvent> {
    match tag {
        Tag::String(s) => Some(NormSoundEvent {
            id: canonical_id(&s.to_string_lossy()),
            range_bits: None,
        }),
        Tag::Compound(c) => {
            let id = canonical_id(&tag_str(c.get(&"sound_id".into())?)?);
            let range_bits = match c.get(&"range".into()) {
                Some(v) => {
                    let f = float_of(v)?;
                    if !f.is_finite() {
                        return None;
                    }
                    Some(f32_bits_key(f))
                }
                None => None,
            };
            Some(NormSoundEvent { id, range_bits })
        }
        _ => None,
    }
}

fn sound_event_equal(a: &Tag, b: &Tag) -> bool {
    matches!((norm_sound_event(a), norm_sound_event(b)), (Some(x), Some(y)) if x == y)
}

fn instrument_equal(a: &Tag, b: &Tag) -> bool {
    match (a, b) {
        (Tag::String(sa), Tag::String(sb)) => {
            canonical_id(&sa.to_string_lossy()) == canonical_id(&sb.to_string_lossy())
        }
        (Tag::Compound(ca), Tag::Compound(cb)) => {
            let sound_ok = match (ca.get(&"sound_event".into()), cb.get(&"sound_event".into())) {
                (Some(sa), Some(sb)) => sound_event_equal(sa, sb),
                _ => false,
            };
            let dur_ok = match (
                ca.get(&"use_duration".into()).and_then(float_of),
                cb.get(&"use_duration".into()).and_then(float_of),
            ) {
                (Some(da), Some(db)) if da.is_finite() && da > 0.0 && db.is_finite() && db > 0.0 => {
                    f32_bits_key(da) == f32_bits_key(db)
                }
                _ => false,
            };
            let range_ok = match (
                ca.get(&"range".into()).and_then(float_of),
                cb.get(&"range".into()).and_then(float_of),
            ) {
                (Some(ra), Some(rb)) if ra.is_finite() && ra > 0.0 && rb.is_finite() && rb > 0.0 => {
                    f32_bits_key(ra) == f32_bits_key(rb)
                }
                _ => false,
            };
            let desc_ok = match (ca.get(&"description".into()), cb.get(&"description".into())) {
                (Some(da), Some(db)) => text_component_equal(da, db),
                _ => false,
            };
            sound_ok && dur_ok && range_ok && desc_ok
        }
        _ => false,
    }
}

fn typed_entity_data_equal(a: &Tag, b: &Tag) -> bool {
    let (Tag::Compound(ca), Tag::Compound(cb)) = (a, b) else {
        return false;
    };
    let norm = |c: &Compound| -> Option<Compound> {
        let id = tag_str(c.get(&"id".into())?)?;
        let mut out = c.clone();
        out.insert("id".into(), Tag::String(canonical_id(&id).as_str().into()));
        Some(out)
    };
    matches!((norm(ca), norm(cb)), (Some(x), Some(y)) if x == y)
}

#[derive(PartialEq, Eq)]
struct NormExplosion {
    shape: String,
    colors: Vec<i32>,
    fade_colors: Vec<i32>,
    has_trail: bool,
    has_twinkle: bool,
}

fn int_list(tag: &Tag) -> Option<Vec<i32>> {
    Some(match tag {
        Tag::IntArray(v) => v.clone(),
        Tag::ByteArray(v) => v.iter().map(|&b| i32::from(b as i8)).collect(),
        Tag::LongArray(v) => v.iter().map(|&l| l as i32).collect(),
        Tag::List(items) => items
            .iter()
            .map(|t| int_of(t).map(|n| n as i32))
            .collect::<Option<Vec<_>>>()?,
        _ => return None,
    })
}

fn norm_firework_explosion(tag: &Tag) -> Option<NormExplosion> {
    let Tag::Compound(c) = tag else {
        return None;
    };
    let shape = c.get(&"shape".into()).and_then(tag_str)?;
    if !matches!(
        shape.as_str(),
        "small_ball" | "large_ball" | "star" | "creeper" | "burst"
    ) {
        return None;
    }
    let colors = match c.get(&"colors".into()) {
        Some(v) => int_list(v)?,
        None => Vec::new(),
    };
    let fade_colors = match c.get(&"fade_colors".into()) {
        Some(v) => int_list(v)?,
        None => Vec::new(),
    };
    let has_trail = match c.get(&"has_trail".into()) {
        Some(v) => component_bool(v)?,
        None => false,
    };
    let has_twinkle = match c.get(&"has_twinkle".into()) {
        Some(v) => component_bool(v)?,
        None => false,
    };
    Some(NormExplosion {
        shape,
        colors,
        fade_colors,
        has_trail,
        has_twinkle,
    })
}

fn norm_fireworks(tag: &Tag) -> Option<(u8, Vec<NormExplosion>)> {
    let Tag::Compound(c) = tag else {
        return None;
    };
    let flight = match c.get(&"flight_duration".into()) {
        Some(v) => unsigned_byte_of(v)?,
        None => 0,
    };
    let explosions = match c.get(&"explosions".into()) {
        Some(v) => list_slice(v)?
            .iter()
            .map(norm_firework_explosion)
            .collect::<Option<Vec<_>>>()?,
        None => Vec::new(),
    };
    Some((flight, explosions))
}

fn norm_filterable_str(tag: &Tag) -> Option<(String, Option<String>)> {
    match tag {
        Tag::String(s) => Some((s.to_string_lossy(), None)),
        Tag::Compound(c) => {
            let raw = c.get(&"raw".into()).and_then(tag_str)?;
            let filtered = match c.get(&"filtered".into()) {
                Some(v) => Some(tag_str(v)?),
                None => None,
            };
            Some((raw, filtered))
        }
        _ => None,
    }
}

fn norm_writable_book(tag: &Tag) -> Option<Vec<(String, Option<String>)>> {
    let Tag::Compound(c) = tag else {
        return None;
    };
    match c.get(&"pages".into()) {
        Some(v) => list_slice(v)?
            .iter()
            .map(norm_filterable_str)
            .collect::<Option<Vec<_>>>(),
        None => Some(Vec::new()),
    }
}

#[derive(PartialEq, Eq)]
struct NormWrittenBook {
    title: (String, Option<String>),
    author: String,
    generation: i64,
    resolved: bool,
    pages: Vec<(NormComponent, Option<NormComponent>)>,
}

fn norm_written_book(tag: &Tag) -> Option<NormWrittenBook> {
    let Tag::Compound(c) = tag else {
        return None;
    };
    let title = norm_filterable_str(c.get(&"title".into())?)?;
    let author = tag_str(c.get(&"author".into())?)?;
    let generation = match c.get(&"generation".into()) {
        Some(v) => {
            let n = int_of(v)?;
            (0..=3).contains(&n).then_some(n)?
        }
        None => 0,
    };
    let resolved = match c.get(&"resolved".into()) {
        Some(v) => component_bool(v)?,
        None => false,
    };
    let pages = match c.get(&"pages".into()) {
        Some(v) => list_slice(v)?
            .iter()
            .map(|p| {
                if let Tag::Compound(pc) = p
                    && let Some(raw) = pc.get(&"raw".into())
                {
                    let raw_c = norm_text_component(raw, 0)?;
                    let filt_c = match pc.get(&"filtered".into()) {
                        Some(f) => Some(norm_text_component(f, 0)?),
                        None => None,
                    };
                    Some((raw_c, filt_c))
                } else {
                    Some((norm_text_component(p, 0)?, None))
                }
            })
            .collect::<Option<Vec<_>>>()?,
        None => Vec::new(),
    };
    Some(NormWrittenBook {
        title,
        author,
        generation,
        resolved,
        pages,
    })
}

fn written_book_equal(a: &Tag, b: &Tag) -> bool {
    matches!((norm_written_book(a), norm_written_book(b)), (Some(x), Some(y)) if x == y)
}

#[derive(PartialEq, Eq)]
enum NormAttrDisplay {
    Default,
    Hidden,
    Override(Box<NormComponent>),
}

#[derive(PartialEq, Eq)]
struct NormAttrEntry {
    attr: String,
    id: String,
    amount_bits: u64,
    operation: String,
    slot: String,
    display: NormAttrDisplay,
}

fn norm_attribute_modifiers(tag: &Tag) -> Option<Vec<NormAttrEntry>> {
    let entries = match tag {
        Tag::Compound(c) => list_slice(c.get(&"modifiers".into())?)?,
        _ => list_slice(tag)?,
    };
    entries
        .iter()
        .map(|entry| {
            let Tag::Compound(c) = entry else {
                return None;
            };
            let attr = canonical_id(&c.get(&"type".into()).and_then(tag_str)?);
            let id = canonical_id(&c.get(&"id".into()).and_then(tag_str)?);
            let amount_bits = f64_bits_key(double_of(c.get(&"amount".into())?)?);
            let operation = c.get(&"operation".into()).and_then(tag_str)?;
            if !matches!(
                operation.as_str(),
                "add_value" | "add_multiplied_base" | "add_multiplied_total"
            ) {
                return None;
            }
            let slot = match c.get(&"slot".into()) {
                Some(v) => {
                    let s = tag_str(v)?;
                    if !matches!(
                        s.as_str(),
                        "any"
                            | "mainhand"
                            | "offhand"
                            | "hand"
                            | "feet"
                            | "legs"
                            | "chest"
                            | "head"
                            | "armor"
                            | "body"
                            | "saddle"
                    ) {
                        return None;
                    }
                    s
                }
                None => "any".to_owned(),
            };
            let display = match c.get(&"display".into()) {
                Some(Tag::Compound(dc)) => match tag_str(dc.get(&"type".into())?)?.as_str() {
                    "default" => NormAttrDisplay::Default,
                    "hidden" => NormAttrDisplay::Hidden,
                    "override" => NormAttrDisplay::Override(Box::new(norm_text_component(
                        dc.get(&"value".into())?,
                        0,
                    )?)),
                    _ => return None,
                },
                Some(_) => return None,
                None => NormAttrDisplay::Default,
            };
            Some(NormAttrEntry {
                attr,
                id,
                amount_bits,
                operation,
                slot,
                display,
            })
        })
        .collect()
}

#[derive(PartialEq, Eq)]
enum NormConsumeEffect {
    ApplyEffects {
        effects: Vec<NormMobEffect>,
        probability_bits: u32,
    },
    RemoveEffects(Vec<String>),
    ClearAllEffects,
    TeleportRandomly(u32),
    PlaySound(NormSoundEvent),
}

fn norm_consume_effect(tag: &Tag) -> Option<NormConsumeEffect> {
    let Tag::Compound(c) = tag else {
        return None;
    };
    let ty = canonical_id(&tag_str(c.get(&"type".into())?)?);
    match ty.as_str() {
        "minecraft:apply_effects" => {
            let effects = list_slice(c.get(&"effects".into())?)?
                .iter()
                .map(norm_mob_effect)
                .collect::<Option<Vec<_>>>()?;
            let prob = match c.get(&"probability".into()) {
                Some(v) => {
                    let p = float_of(v)?;
                    if p.is_sign_negative() || !(0.0..=1.0).contains(&p) {
                        return None;
                    }
                    p
                }
                None => 1.0_f32,
            };
            Some(NormConsumeEffect::ApplyEffects {
                effects,
                probability_bits: f32_bits_key(prob),
            })
        }
        "minecraft:remove_effects" => {
            let effects = norm_holder_set(c.get(&"effects".into())?)?;
            Some(NormConsumeEffect::RemoveEffects(effects))
        }
        "minecraft:clear_all_effects" => Some(NormConsumeEffect::ClearAllEffects),
        "minecraft:teleport_randomly" => {
            let diam = match c.get(&"diameter".into()) {
                Some(v) => {
                    let d = float_of(v)?;
                    if !d.is_finite() || d <= 0.0 {
                        return None;
                    }
                    d
                }
                None => 16.0_f32,
            };
            Some(NormConsumeEffect::TeleportRandomly(f32_bits_key(diam)))
        }
        "minecraft:play_sound" => {
            let sound = norm_sound_event(c.get(&"sound".into())?)?;
            Some(NormConsumeEffect::PlaySound(sound))
        }
        _ => None,
    }
}

fn death_protection_equal(a: &Tag, b: &Tag) -> bool {
    let norm = |tag: &Tag| -> Option<Vec<NormConsumeEffect>> {
        let Tag::Compound(c) = tag else {
            return None;
        };
        match c.get(&"death_effects".into()) {
            Some(v) => list_slice(v)?
                .iter()
                .map(norm_consume_effect)
                .collect::<Option<Vec<_>>>(),
            None => Some(Vec::new()),
        }
    };
    matches!((norm(a), norm(b)), (Some(x), Some(y)) if x == y)
}

#[derive(PartialEq, Eq)]
struct NormConsumable {
    seconds_bits: u32,
    animation: String,
    sound: NormSoundEvent,
    has_particles: bool,
    effects: Vec<NormConsumeEffect>,
}

fn norm_consumable(tag: &Tag) -> Option<NormConsumable> {
    let Tag::Compound(c) = tag else {
        return None;
    };
    let seconds = match c.get(&"consume_seconds".into()) {
        Some(v) => {
            let s = float_of(v)?;
            if !s.is_finite() || s.is_sign_negative() {
                return None;
            }
            s
        }
        None => 1.6_f32,
    };
    let animation = match c.get(&"animation".into()) {
        Some(v) => {
            let anim = tag_str(v)?;
            if !matches!(
                anim.as_str(),
                "none"
                    | "eat"
                    | "drink"
                    | "block"
                    | "bow"
                    | "trident"
                    | "crossbow"
                    | "spyglass"
                    | "toot_horn"
                    | "brush"
                    | "bundle"
                    | "spear"
            ) {
                return None;
            }
            anim
        }
        None => "eat".to_owned(),
    };
    let sound = match c.get(&"sound".into()) {
        Some(v) => norm_sound_event(v)?,
        None => NormSoundEvent {
            id: "minecraft:entity.generic.eat".to_owned(),
            range_bits: None,
        },
    };
    let has_particles = match c.get(&"has_consume_particles".into()) {
        Some(v) => component_bool(v)?,
        None => true,
    };
    let effects = match c.get(&"on_consume_effects".into()) {
        Some(v) => list_slice(v)?
            .iter()
            .map(norm_consume_effect)
            .collect::<Option<Vec<_>>>()?,
        None => Vec::new(),
    };
    Some(NormConsumable {
        seconds_bits: f32_bits_key(seconds),
        animation,
        sound,
        has_particles,
        effects,
    })
}

fn consumable_equal(a: &Tag, b: &Tag) -> bool {
    matches!((norm_consumable(a), norm_consumable(b)), (Some(x), Some(y)) if x == y)
}

#[derive(PartialEq, Eq)]
struct NormEquippable {
    slot: String,
    equip_sound: NormSoundEvent,
    shearing_sound: NormSoundEvent,
    asset_id: Option<String>,
    camera_overlay: Option<String>,
    allowed_entities: Option<Vec<String>>,
    flags: [bool; 5],
}

fn norm_equippable(tag: &Tag) -> Option<NormEquippable> {
    let Tag::Compound(c) = tag else {
        return None;
    };
    let slot = tag_str(c.get(&"slot".into())?)?;
    if !matches!(
        slot.as_str(),
        "mainhand" | "offhand" | "feet" | "legs" | "chest" | "head" | "body" | "saddle"
    ) {
        return None;
    }
    let equip_sound = match c.get(&"equip_sound".into()) {
        Some(v) => norm_sound_event(v)?,
        None => NormSoundEvent {
            id: "minecraft:item.armor.equip_generic".to_owned(),
            range_bits: None,
        },
    };
    let shearing_sound = match c.get(&"shearing_sound".into()) {
        Some(v) => norm_sound_event(v)?,
        None => NormSoundEvent {
            id: "minecraft:item.shears.snip".to_owned(),
            range_bits: None,
        },
    };
    let asset_id = match c.get(&"asset_id".into()) {
        Some(v) => Some(canonical_id(&tag_str(v)?)),
        None => None,
    };
    let camera_overlay = match c.get(&"camera_overlay".into()) {
        Some(v) => Some(canonical_id(&tag_str(v)?)),
        None => None,
    };
    let allowed_entities = match c.get(&"allowed_entities".into()) {
        Some(v) => Some(norm_holder_set(v)?),
        None => None,
    };
    let mut flags = [true, true, true, false, false];
    for (slot_flag, (key, default)) in flags.iter_mut().zip([
        ("dispensable", true),
        ("swappable", true),
        ("damage_on_hurt", true),
        ("equip_on_interact", false),
        ("can_be_sheared", false),
    ]) {
        *slot_flag = match c.get(&JavaString::from(key)) {
            Some(v) => component_bool(v)?,
            None => default,
        };
    }
    Some(NormEquippable {
        slot,
        equip_sound,
        shearing_sound,
        asset_id,
        camera_overlay,
        allowed_entities,
        flags,
    })
}

fn equippable_equal(a: &Tag, b: &Tag) -> bool {
    matches!((norm_equippable(a), norm_equippable(b)), (Some(x), Some(y)) if x == y)
}

#[derive(PartialEq, Eq)]
struct NormToolRule {
    blocks: Vec<String>,
    speed_bits: Option<u32>,
    correct_for_drops: Option<bool>,
}

#[derive(PartialEq, Eq)]
struct NormTool {
    default_mining_speed_bits: u32,
    damage_per_block: i64,
    can_destroy_in_creative: bool,
    rules: Vec<NormToolRule>,
}

fn norm_tool(tag: &Tag) -> Option<NormTool> {
    let Tag::Compound(c) = tag else {
        return None;
    };
    let default_speed = match c.get(&"default_mining_speed".into()) {
        Some(v) => {
            let s = float_of(v)?;
            if !s.is_finite() {
                return None;
            }
            s
        }
        None => 1.0_f32,
    };
    let damage_per_block = match c.get(&"damage_per_block".into()) {
        Some(v) => {
            let n = int_of(v)?;
            (n >= 0).then_some(n)?
        }
        None => 1,
    };
    let can_destroy_in_creative = match c.get(&"can_destroy_blocks_in_creative".into()) {
        Some(v) => component_bool(v)?,
        None => true,
    };
    let rules = list_slice(c.get(&"rules".into())?)?
        .iter()
        .map(|entry| {
            let Tag::Compound(rc) = entry else {
                return None;
            };
            let blocks = norm_holder_set(rc.get(&"blocks".into())?)?;
            let speed_bits = match rc.get(&"speed".into()) {
                Some(v) => {
                    let s = float_of(v)?;
                    if !s.is_finite() || s <= 0.0 {
                        return None;
                    }
                    Some(f32_bits_key(s))
                }
                None => None,
            };
            let correct_for_drops = match rc.get(&"correct_for_drops".into()) {
                Some(v) => Some(component_bool(v)?),
                None => None,
            };
            Some(NormToolRule {
                blocks,
                speed_bits,
                correct_for_drops,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(NormTool {
        default_mining_speed_bits: f32_bits_key(default_speed),
        damage_per_block,
        can_destroy_in_creative,
        rules,
    })
}

fn tool_equal(a: &Tag, b: &Tag) -> bool {
    matches!((norm_tool(a), norm_tool(b)), (Some(x), Some(y)) if x == y)
}

/// Vanilla `MobEffectInstance.equals`: compares effect, amplifier, duration,
/// ambient, visible (`show_particles`), and `show_icon` (which defaults to
/// `show_particles`), while ignoring `hidden_effect`.
#[derive(PartialEq, Eq)]
struct NormMobEffect {
    id: String,
    amplifier: u8,
    duration: i32,
    ambient: bool,
    show_particles: bool,
    show_icon: bool,
}

fn validate_mob_effect_details(
    tag: &Tag,
    depth: usize,
) -> Option<(u8, i32, bool, bool, bool)> {
    if depth > MAX_NESTING_DEPTH {
        return None;
    }
    let Tag::Compound(c) = tag else {
        return None;
    };
    let amplifier = match c.get(&"amplifier".into()) {
        Some(v) => unsigned_byte_of(v)?,
        None => 0,
    };
    let duration = match c.get(&"duration".into()) {
        Some(v) => int_of(v)? as i32,
        None => 0,
    };
    let ambient = match c.get(&"ambient".into()) {
        Some(v) => component_bool(v)?,
        None => false,
    };
    let show_particles = match c.get(&"show_particles".into()) {
        Some(v) => component_bool(v)?,
        None => true,
    };
    let show_icon = match c.get(&"show_icon".into()) {
        Some(v) => component_bool(v)?,
        None => show_particles,
    };
    if let Some(hidden) = c.get(&"hidden_effect".into()) {
        validate_mob_effect_details(hidden, depth + 1)?;
    }
    Some((amplifier, duration, ambient, show_particles, show_icon))
}

fn norm_mob_effect(tag: &Tag) -> Option<NormMobEffect> {
    let Tag::Compound(c) = tag else {
        return None;
    };
    let id = canonical_id(&tag_str(c.get(&"id".into())?)?);
    let (amplifier, duration, ambient, show_particles, show_icon) =
        validate_mob_effect_details(tag, 0)?;
    Some(NormMobEffect {
        id,
        amplifier,
        duration,
        ambient,
        show_particles,
        show_icon,
    })
}

#[derive(PartialEq, Eq)]
struct NormPotionContents {
    potion: Option<String>,
    custom_color: Option<i32>,
    custom_effects: Vec<NormMobEffect>,
    custom_name: Option<String>,
}

fn norm_potion_contents(tag: &Tag) -> Option<NormPotionContents> {
    match tag {
        Tag::String(s) => Some(NormPotionContents {
            potion: Some(canonical_id(&s.to_string_lossy())),
            custom_color: None,
            custom_effects: Vec::new(),
            custom_name: None,
        }),
        Tag::Compound(c) => {
            let potion = match c.get(&"potion".into()) {
                Some(v) => Some(canonical_id(&tag_str(v)?)),
                None => None,
            };
            let custom_color = match c.get(&"custom_color".into()) {
                Some(v) => Some(int_of(v)? as i32),
                None => None,
            };
            let custom_effects = match c.get(&"custom_effects".into()) {
                Some(v) => list_slice(v)?
                    .iter()
                    .map(norm_mob_effect)
                    .collect::<Option<Vec<_>>>()?,
                None => Vec::new(),
            };
            let custom_name = match c.get(&"custom_name".into()) {
                Some(v) => Some(tag_str(v)?),
                None => None,
            };
            Some(NormPotionContents {
                potion,
                custom_color,
                custom_effects,
                custom_name,
            })
        }
        _ => None,
    }
}

fn potion_contents_equal(a: &Tag, b: &Tag) -> bool {
    matches!(
        (norm_potion_contents(a), norm_potion_contents(b)),
        (Some(x), Some(y)) if x == y
    )
}

fn norm_stew_effects(tag: &Tag) -> Option<Vec<(String, i32)>> {
    list_slice(tag)?
        .iter()
        .map(|entry| {
            let Tag::Compound(c) = entry else {
                return None;
            };
            let id = canonical_id(&c.get(&"id".into()).and_then(tag_str)?);
            let dur = match c.get(&"duration".into()) {
                Some(v) => int_of(v)? as i32,
                None => 160,
            };
            Some((id, dur))
        })
        .collect()
}

fn norm_banner_patterns(tag: &Tag) -> Option<Vec<(Tag, String)>> {
    list_slice(tag)?
        .iter()
        .map(|entry| {
            let Tag::Compound(c) = entry else {
                return None;
            };
            let pattern = match c.get(&"pattern".into())? {
                Tag::String(s) => Tag::String(canonical_id(&s.to_string_lossy()).as_str().into()),
                Tag::Compound(pc) => Tag::compound([
                    (
                        "asset_id",
                        Tag::String(
                            canonical_id(&tag_str(pc.get(&"asset_id".into())?)?)
                                .as_str()
                                .into(),
                        ),
                    ),
                    (
                        "translation_key",
                        Tag::String(tag_str(pc.get(&"translation_key".into())?)?.as_str().into()),
                    ),
                ]),
                _ => return None,
            };
            let color = c.get(&"color".into()).and_then(tag_str)?;
            if !horizon_items::firework::DYE_FIREWORK_COLORS
                .iter()
                .any(|(name, _)| *name == color)
            {
                return None;
            }
            Some((pattern, color))
        })
        .collect()
}

#[derive(PartialEq, Eq)]
enum NormTrimMaterial {
    Named(String),
    Direct {
        palette_id: String,
        description: NormComponent,
    },
}

#[derive(PartialEq, Eq)]
enum NormTrimPattern {
    Named(String),
    Direct {
        asset_id: String,
        description: NormComponent,
        decal: bool,
    },
}

fn norm_trim_material(tag: &Tag) -> Option<NormTrimMaterial> {
    match tag {
        Tag::String(s) => Some(NormTrimMaterial::Named(canonical_id(&s.to_string_lossy()))),
        Tag::Compound(c) => Some(NormTrimMaterial::Direct {
            palette_id: canonical_id(&tag_str(c.get(&"palette_id".into())?)?),
            description: norm_text_component(c.get(&"description".into())?, 0)?,
        }),
        _ => None,
    }
}

fn norm_trim_pattern(tag: &Tag) -> Option<NormTrimPattern> {
    match tag {
        Tag::String(s) => Some(NormTrimPattern::Named(canonical_id(&s.to_string_lossy()))),
        Tag::Compound(c) => {
            let decal = match c.get(&"decal".into()) {
                Some(v) => component_bool(v)?,
                None => false,
            };
            Some(NormTrimPattern::Direct {
                asset_id: canonical_id(&tag_str(c.get(&"asset_id".into())?)?),
                description: norm_text_component(c.get(&"description".into())?, 0)?,
                decal,
            })
        }
        _ => None,
    }
}

fn trim_equal(a: &Tag, b: &Tag) -> bool {
    let norm = |tag: &Tag| -> Option<(NormTrimMaterial, NormTrimPattern)> {
        let Tag::Compound(c) = tag else {
            return None;
        };
        Some((
            norm_trim_material(c.get(&"material".into())?)?,
            norm_trim_pattern(c.get(&"pattern".into())?)?,
        ))
    };
    matches!((norm(a), norm(b)), (Some(x), Some(y)) if x == y)
}

fn pot_decorations_equal(env: &PredicateEnv<'_>, a: &Tag, b: &Tag, depth: usize) -> bool {
    let (Tag::Compound(ca), Tag::Compound(cb)) = (a, b) else {
        return false;
    };
    ["back", "left", "right", "front"].iter().all(|side| {
        let key = JavaString::from(*side);
        match (ca.get(&key), cb.get(&key)) {
            (None, None) => true,
            (Some(sa), Some(sb)) => item_template_equal(env, sa, sb, depth + 1),
            _ => false,
        }
    })
}

fn norm_holder_set(tag: &Tag) -> Option<Vec<String>> {
    match tag {
        Tag::String(s) => {
            let raw = s.to_string_lossy();
            if let Some(t) = raw.strip_prefix('#') {
                Some(vec![format!("#{}", canonical_id(t))])
            } else {
                Some(vec![canonical_id(&raw)])
            }
        }
        _ => list_slice(tag)?
            .iter()
            .map(|t| tag_str(t).map(|s| canonical_id(&s)))
            .collect::<Option<Vec<_>>>(),
    }
}

fn holder_set_equal(a: &Tag, b: &Tag) -> bool {
    matches!((norm_holder_set(a), norm_holder_set(b)), (Some(x), Some(y)) if x == y)
}

fn item_template_equal(env: &PredicateEnv<'_>, a: &Tag, b: &Tag, depth: usize) -> bool {
    let (Tag::Compound(ca), Tag::Compound(cb)) = (a, b) else {
        return false;
    };
    let (Some(id_a), Some(id_b)) = (
        ca.get(&"id".into())
            .and_then(tag_str)
            .map(|s| canonical_id(&s)),
        cb.get(&"id".into())
            .and_then(tag_str)
            .map(|s| canonical_id(&s)),
    ) else {
        return false;
    };
    if id_a != id_b {
        return false;
    }
    let read_count = |c: &Compound| -> Option<i64> {
        match c.get(&"count".into()) {
            Some(v) => {
                let n = int_of(v)?;
                (1..=99).contains(&n).then_some(n)
            }
            None => Some(1),
        }
    };
    let (Some(count_a), Some(count_b)) = (read_count(ca), read_count(cb)) else {
        return false;
    };
    if count_a != count_b {
        return false;
    }
    let proto = env.items.component_defaults.get(&id_a);
    let build_eff = |c: &Compound| -> Option<Compound> {
        let mut eff = proto.cloned().unwrap_or_default();
        if let Some(comps) = c.get(&"components".into()) {
            let Tag::Compound(patch) = comps else {
                return None;
            };
            for (k, v) in patch {
                let ks = k.to_string_lossy();
                if let Some(rem) = ks.strip_prefix('!') {
                    eff.remove(&JavaString::from(canonical_id(rem).as_str()));
                } else {
                    eff.insert(JavaString::from(canonical_id(&ks).as_str()), v.clone());
                }
            }
        }
        Some(eff)
    };
    let (Some(eff_a), Some(eff_b)) = (build_eff(ca), build_eff(cb)) else {
        return false;
    };
    if eff_a.len() != eff_b.len() {
        return false;
    }
    eff_a.iter().all(|(k, va)| {
        let ks = k.to_string_lossy();
        eff_b
            .get(k)
            .is_some_and(|vb| exact_component_equals(env, &ks, va, vb, depth + 1))
    })
}

fn extract_container_slots(list: &[Tag]) -> Option<std::collections::BTreeMap<i64, &Tag>> {
    let mut map = std::collections::BTreeMap::new();
    for entry in list {
        let Tag::Compound(c) = entry else {
            return None;
        };
        let slot = int_of(c.get(&"slot".into())?)?;
        if !(0..=255).contains(&slot) {
            return None;
        }
        let item = c.get(&"item".into())?;
        map.insert(slot, item);
    }
    Some(map)
}

fn container_equal(env: &PredicateEnv<'_>, a: &Tag, b: &Tag, depth: usize) -> bool {
    let (Some(la), Some(lb)) = (list_slice(a), list_slice(b)) else {
        return false;
    };
    let (Some(ma), Some(mb)) = (extract_container_slots(la), extract_container_slots(lb)) else {
        return false;
    };
    ma.len() == mb.len()
        && ma.iter().all(|(slot, ia)| {
            mb.get(slot)
                .is_some_and(|ib| item_template_equal(env, ia, ib, depth + 1))
        })
}

fn bees_equal(a: &Tag, b: &Tag) -> bool {
    let (Some(la), Some(lb)) = (list_slice(a), list_slice(b)) else {
        return false;
    };
    if la.len() != lb.len() {
        return false;
    }
    la.iter().zip(lb).all(|(x, y)| {
        let (Tag::Compound(cx), Tag::Compound(cy)) = (x, y) else {
            return false;
        };
        let entity_ok = match (cx.get(&"entity_data".into()), cy.get(&"entity_data".into())) {
            (Some(ex), Some(ey)) => typed_entity_data_equal(ex, ey),
            _ => false,
        };
        let ticks_ok = matches!(
            (
                cx.get(&"ticks_in_hive".into()).and_then(int_of),
                cy.get(&"ticks_in_hive".into()).and_then(int_of),
            ),
            (Some(ta), Some(tb)) if ta == tb
        );
        let min_ok = matches!(
            (
                cx.get(&"min_ticks_in_hive".into()).and_then(int_of),
                cy.get(&"min_ticks_in_hive".into()).and_then(int_of),
            ),
            (Some(ma), Some(mb)) if ma == mb
        );
        entity_ok && ticks_ok && min_ok
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn snbt(s: &str) -> Tag {
        parse_fully(s).unwrap_or_else(|e| panic!("bad snbt {s}: {}", e.message.plain()))
    }

    fn compound(s: &str) -> Compound {
        match snbt(s) {
            Tag::Compound(c) => c,
            other => panic!("expected compound: {other:?}"),
        }
    }

    fn test_items() -> Items {
        let common = r#"{
            "minecraft:max_stack_size": 64,
            "minecraft:lore": [],
            "minecraft:enchantments": {},
            "minecraft:repair_cost": 0,
            "minecraft:attribute_modifiers": [],
            "minecraft:rarity": "common",
            "minecraft:tooltip_display": {},
            "minecraft:use_effects": {}
        }"#;
        let with_extra = |extra: &str| {
            let mut base = compound(common);
            for (k, v) in compound(extra) {
                base.insert(k, v);
            }
            base
        };
        let mut defaults: HashMap<String, Compound> = HashMap::new();
        defaults.insert("minecraft:stone".into(), compound(common));
        defaults.insert(
            "minecraft:firework_rocket".into(),
            with_extra(r#"{"minecraft:fireworks": {flight_duration: 1b}}"#),
        );
        defaults.insert(
            "minecraft:diamond_sword".into(),
            with_extra(
                r#"{
                    "minecraft:max_stack_size": 1,
                    "minecraft:damage": 0,
                    "minecraft:max_damage": 1561,
                    "minecraft:attribute_modifiers": [
                        {type: "minecraft:attack_damage", id: "minecraft:base_attack_damage", amount: 6.0d, operation: "add_value", slot: "mainhand"}
                    ]
                }"#,
            ),
        );
        defaults.insert(
            "minecraft:shulker_box".into(),
            with_extra(r#"{"minecraft:max_stack_size": 1, "minecraft:container": []}"#),
        );
        defaults.insert(
            "minecraft:bundle".into(),
            with_extra(r#"{"minecraft:max_stack_size": 1, "minecraft:bundle_contents": []}"#),
        );
        defaults.insert(
            "minecraft:writable_book".into(),
            with_extra(
                r#"{"minecraft:max_stack_size": 1, "minecraft:writable_book_content": {}}"#,
            ),
        );
        defaults.insert(
            "minecraft:enchanted_book".into(),
            with_extra(
                r#"{"minecraft:max_stack_size": 1, "minecraft:stored_enchantments": {}}"#,
            ),
        );
        defaults.insert(
            "minecraft:music_disc_cat".into(),
            with_extra(r#"{"minecraft:max_stack_size": 1, "minecraft:jukebox_playable": "minecraft:cat"}"#),
        );
        defaults.insert(
            "minecraft:potion".into(),
            with_extra(r#"{"minecraft:max_stack_size": 1, "minecraft:potion_contents": {}}"#),
        );
        let mut items = Items::empty();
        items.component_defaults = Arc::new(defaults);
        items
    }

    fn stack_with(item: &str, count: u32, patch_snbt: Option<&str>) -> ItemStack {
        let (damage, extra) = match patch_snbt {
            Some(p) => {
                let tag = snbt(p);
                horizon_items::item_data::decode_components_for_item(item, Some(&tag))
            }
            None => (0, None),
        };
        ItemStack {
            item: Arc::from(item),
            count,
            damage,
            extra,
        }
    }

    fn pred(components: &str) -> ItemInput {
        ItemInput {
            id: "*".into(),
            tag: false,
            any: true,
            components: Some(components.into()),
        }
    }

    #[test]
    fn sub_predicates_on_plain_and_patched_stacks_match_vanilla_26_3() {
        let items = test_items();
        let registries = Registries::default();
        let no_tags = |_: &str, _: &str| Vec::new();
        let env = PredicateEnv {
            items: &items,
            registries: &registries,
            tag_lookup: &no_tags,
        };

        // 1. custom_data~{}: CustomDataPredicate defaults absent component to CustomData.EMPTY.
        let plain_stone = stack_with("minecraft:stone", 1, None);
        assert!(stack_matches_in(&env, &pred("[minecraft:custom_data~{}]"), &plain_stone));
        assert!(stack_matches_in(&env, &pred(r#"[minecraft:custom_data~"{}"]"#), &plain_stone));
        assert!(!stack_matches_in(&env, &pred("[minecraft:custom_data~{a:1b}]"), &plain_stone));
        let tagged_stone = stack_with(
            "minecraft:stone",
            1,
            Some(r#"{"minecraft:custom_data":{a:1b,b:2}}"#),
        );
        assert!(stack_matches_in(&env, &pred("[minecraft:custom_data~{a:1b}]"), &tagged_stone));
        assert!(!stack_matches_in(&env, &pred("[minecraft:custom_data~{a:1}]"), &tagged_stone));

        // 2. enchantments~[] and stored_enchantments~[]: plain stack has prototype enchantments={}.
        assert!(stack_matches_in(&env, &pred("[minecraft:enchantments~[]]"), &plain_stone));
        assert!(!stack_matches_in(&env, &pred("[minecraft:stored_enchantments~[]]"), &plain_stone));
        let plain_ench_book = stack_with("minecraft:enchanted_book", 1, None);
        assert!(stack_matches_in(&env, &pred("[minecraft:stored_enchantments~[]]"), &plain_ench_book));
        let no_ench_stone = stack_with(
            "minecraft:stone",
            1,
            Some(r#"{"!minecraft:enchantments":{}}"#),
        );
        assert!(!stack_matches_in(&env, &pred("[minecraft:enchantments~[]]"), &no_ench_stone));
        let sharp_sword = stack_with(
            "minecraft:diamond_sword",
            1,
            Some(r#"{"minecraft:enchantments":{"minecraft:sharpness":3}}"#),
        );
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:enchantments~[{enchantments:"minecraft:sharpness",levels:{min:2}}]]"#),
            &sharp_sword
        ));
        assert!(!stack_matches_in(
            &env,
            &pred(r#"[minecraft:enchantments~[{enchantments:"minecraft:sharpness",levels:{min:4}}]]"#),
            &sharp_sword
        ));
        assert!(stack_matches_in(&env, &pred("[minecraft:enchantments~[{}]]"), &sharp_sword));
        assert!(!stack_matches_in(&env, &pred("[minecraft:enchantments~[{}]]"), &plain_stone));

        // 3. damage~: both damage and durability bounds, respecting prototype and !damage removal.
        let plain_sword = stack_with("minecraft:diamond_sword", 1, None);
        assert!(stack_matches_in(&env, &pred("[minecraft:damage~{damage:0}]"), &plain_sword));
        assert!(stack_matches_in(&env, &pred("[minecraft:damage~{durability:1561}]"), &plain_sword));
        assert!(stack_matches_in(&env, &pred("[minecraft:damage~{durability:{min:1000}}]"), &plain_sword));
        assert!(!stack_matches_in(&env, &pred("[minecraft:damage~{damage:0}]"), &plain_stone));
        let worn_sword = stack_with(
            "minecraft:diamond_sword",
            1,
            Some(r#"{"minecraft:damage":100,"minecraft:max_damage":250}"#),
        );
        assert!(stack_matches_in(
            &env,
            &pred("[minecraft:damage~{damage:100,durability:150}]"),
            &worn_sword
        ));
        let no_dmg_sword = stack_with(
            "minecraft:diamond_sword",
            1,
            Some(r#"{"!minecraft:damage":{}}"#),
        );
        assert!(!stack_matches_in(&env, &pred("[minecraft:damage~{}]"), &no_dmg_sword));

        // 4. fireworks~ and firework_explosion~: prototype rocket has flight_duration=1, explosions=[].
        let plain_rocket = stack_with("minecraft:firework_rocket", 1, None);
        assert!(stack_matches_in(&env, &pred("[minecraft:fireworks~{flight_duration:1}]"), &plain_rocket));
        assert!(stack_matches_in(&env, &pred("[minecraft:fireworks~{explosions:{size:0}}]"), &plain_rocket));
        assert!(!stack_matches_in(&env, &pred("[minecraft:fireworks~{flight_duration:2}]"), &plain_rocket));
        let star_rocket = stack_with(
            "minecraft:firework_rocket",
            1,
            Some(r#"{"minecraft:fireworks":{flight_duration:2b,explosions:[{shape:"star",has_twinkle:1b}]}}"#),
        );
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:fireworks~{flight_duration:2,explosions:{contains:[{shape:"star",has_twinkle:true,has_trail:false}]}}]"#),
            &star_rocket
        ));
        let star = stack_with(
            "minecraft:firework_star",
            1,
            Some(r#"{"minecraft:firework_explosion":{shape:"creeper",has_trail:1b}}"#),
        );
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:firework_explosion~{shape:"creeper",has_trail:true,has_twinkle:false}]"#),
            &star
        ));

        // 5. container~ and bundle_contents~: empty prototypes & nested ItemPredicates.
        let plain_shulker = stack_with("minecraft:shulker_box", 1, None);
        assert!(stack_matches_in(&env, &pred("[minecraft:container~{items:{size:0}}]"), &plain_shulker));
        assert!(!stack_matches_in(&env, &pred("[minecraft:container~{items:{size:{min:1}}}]"), &plain_shulker));
        assert!(!stack_matches_in(&env, &pred("[minecraft:container~{items:{size:0}}]"), &plain_stone));
        let filled_shulker = stack_with(
            "minecraft:shulker_box",
            1,
            Some(r#"{"minecraft:container":[{slot:0,item:{id:"minecraft:stone",count:3}}]}"#),
        );
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:container~{items:{size:1,contains:[{items:"minecraft:stone",count:3}]}}]"#),
            &filled_shulker
        ));
        let filled_bundle = stack_with(
            "minecraft:bundle",
            1,
            Some(r#"{"minecraft:bundle_contents":[{id:"minecraft:stone",count:2}]}"#),
        );
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:bundle_contents~{items:{count:[{test:{items:"minecraft:stone"},count:1}]}}]"#),
            &filled_bundle
        ));

        // 6. attribute_modifiers~: plain stacks have prototype []; diamond_sword has attack_damage.
        assert!(stack_matches_in(&env, &pred("[minecraft:attribute_modifiers~{modifiers:{size:0}}]"), &plain_stone));
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:attribute_modifiers~{modifiers:{size:{min:1},contains:[{attribute:"minecraft:attack_damage",amount:6.0d,operation:"add_value",slot:"mainhand"}]}}]"#),
            &plain_sword
        ));

        // 7. writable_book_content~ and written_book_content~.
        let plain_book = stack_with("minecraft:writable_book", 1, None);
        assert!(stack_matches_in(&env, &pred("[minecraft:writable_book_content~{pages:{size:0}}]"), &plain_book));
        let draft_book = stack_with(
            "minecraft:writable_book",
            1,
            Some(r#"{"minecraft:writable_book_content":{pages:["hello"]}}"#),
        );
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:writable_book_content~{pages:{contains:["hello"]}}]"#),
            &draft_book
        ));
        let written = stack_with(
            "minecraft:written_book",
            1,
            Some(r#"{"minecraft:written_book_content":{title:"Tales",author:"Alex",pages:["Page 1"]}}"#),
        );
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:written_book_content~{author:"Alex",title:"Tales",generation:0,resolved:false,pages:{contains:["Page 1"]}}]"#),
            &written
        ));

        // 8. potion_contents~ (26.3 `{potions, effects}`), trim~, jukebox_playable~, villager/variant~.
        let swift_potion = stack_with(
            "minecraft:potion",
            1,
            Some(r#"{"minecraft:potion_contents":{potion:"minecraft:swiftness"}}"#),
        );
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:potion_contents~{potions:"minecraft:swiftness",effects:{contains:[{"minecraft:speed":{amplifier:0}}]}}]"#),
            &swift_potion
        ));
        let trimmed = stack_with(
            "minecraft:diamond_sword",
            1,
            Some(r#"{"minecraft:trim":{material:"minecraft:iron",pattern:"minecraft:coast"}}"#),
        );
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:trim~{material:"minecraft:iron",pattern:"minecraft:coast"}]"#),
            &trimmed
        ));
        let disc = stack_with("minecraft:music_disc_cat", 1, None);
        assert!(stack_matches_in(&env, &pred(r#"[minecraft:jukebox_playable~{song:"minecraft:cat"}]"#), &disc));
        let egg = stack_with(
            "minecraft:stone",
            1,
            Some(r#"{"minecraft:villager/variant":"minecraft:plains"}"#),
        );
        assert!(stack_matches_in(&env, &pred(r#"[minecraft:villager/variant~"minecraft:plains"]"#), &egg));

        // 9. Existence predicates (`comp~{}`) and `count` pseudo-component presence.
        assert!(stack_matches_in(&env, &pred("[minecraft:max_stack_size~{}]"), &plain_stone));
        assert!(stack_matches_in(&env, &pred("[count]"), &plain_stone));
        assert!(!stack_matches_in(&env, &pred("[!count]"), &plain_stone));
        assert!(stack_matches_in(&env, &pred("[count=1b]"), &plain_stone));
        let removed_use_effects = stack_with(
            "minecraft:stone",
            1,
            Some(r#"{"!minecraft:use_effects":{}}"#),
        );
        assert!(stack_matches_in(&env, &pred("[!minecraft:use_effects]"), &removed_use_effects));
        assert!(!stack_matches_in(&env, &pred("[minecraft:use_effects~{}]"), &removed_use_effects));
    }

    #[test]
    fn exact_component_predicates_compare_decoded_effective_values() {
        let items = test_items();
        let registries = Registries::default();
        let no_tags = |_: &str, _: &str| Vec::new();
        let env = PredicateEnv {
            items: &items,
            registries: &registries,
            tag_lookup: &no_tags,
        };

        // Plain firework rocket vs explicit default explosions & Int flight_duration.
        let plain_rocket = stack_with("minecraft:firework_rocket", 1, None);
        assert!(stack_matches_in(
            &env,
            &pred("[minecraft:fireworks={flight_duration:1,explosions:[]}]"),
            &plain_rocket
        ));
        assert!(stack_matches_in(
            &env,
            &pred("[minecraft:fireworks={flight_duration:1b}]"),
            &plain_rocket
        ));
        assert!(!stack_matches_in(
            &env,
            &pred("[minecraft:fireworks={flight_duration:2,explosions:[]}]"),
            &plain_rocket
        ));

        // Plain writable_book prototype `{}` equals `{pages:[]}`.
        let plain_book = stack_with("minecraft:writable_book", 1, None);
        assert!(stack_matches_in(
            &env,
            &pred("[minecraft:writable_book_content={pages:[]}]"),
            &plain_book
        ));

        // Attribute modifiers: Int amount vs Double amount and omitted slot vs slot:"any".
        let armor_item = stack_with(
            "minecraft:stone",
            1,
            Some(r#"{"minecraft:attribute_modifiers":[{type:"minecraft:armor",amount:1.0d,id:"minecraft:fz",operation:"add_value"}]}"#),
        );
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:attribute_modifiers=[{type:"armor",amount:1,id:"fz",operation:"add_value",slot:"any"}]]"#),
            &armor_item
        ));

        // Death protection `{}` equals `{death_effects:[]}`.
        let dp = stack_with("minecraft:stone", 1, Some(r#"{"minecraft:death_protection":{}}"#));
        assert!(stack_matches_in(
            &env,
            &pred("[minecraft:death_protection={death_effects:[]}]"),
            &dp
        ));

        // Food, consumable, potion_contents, suspicious_stew_effects defaults.
        let food = stack_with(
            "minecraft:stone",
            1,
            Some(r#"{"minecraft:food":{nutrition:2,saturation:0.5f}}"#),
        );
        assert!(stack_matches_in(
            &env,
            &pred("[minecraft:food={nutrition:2,saturation:0.5d,can_always_eat:false}]"),
            &food
        ));
        let potion = stack_with(
            "minecraft:potion",
            1,
            Some(r#"{"minecraft:potion_contents":"minecraft:swiftness"}"#),
        );
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:potion_contents={potion:"swiftness",custom_effects:[]}]"#),
            &potion
        ));

        // Raw custom_data stays tag-type strict (`1b != 1`).
        let custom = stack_with("minecraft:stone", 1, Some(r#"{"minecraft:custom_data":{a:1b}}"#));
        assert!(stack_matches_in(&env, &pred("[minecraft:custom_data={a:1b}]"), &custom));
        assert!(!stack_matches_in(&env, &pred("[minecraft:custom_data={a:1}]"), &custom));

        // Fireworks flight_duration 128..=255 survives Horizon's signed Tag::Byte serialization
        // in both `~` and `=`.
        let high_flight = stack_with(
            "minecraft:firework_rocket",
            1,
            Some(r#"{"minecraft:fireworks":{flight_duration:200}}"#),
        );
        assert!(stack_matches_in(
            &env,
            &pred("[minecraft:fireworks~{flight_duration:200}]"),
            &high_flight
        ));
        assert!(stack_matches_in(
            &env,
            &pred("[minecraft:fireworks={flight_duration:200,explosions:[]}]"),
            &high_flight
        ));
        assert!(!stack_matches_in(
            &env,
            &pred("[minecraft:fireworks~{flight_duration:-56}]"),
            &high_flight
        ));

        // Flattened SNBT string custom_data patches match `custom_data~`.
        let flattened_custom = stack_with(
            "minecraft:stone",
            1,
            Some(r#"{"minecraft:custom_data":"{a:1b,b:2}"}"#),
        );
        assert!(stack_matches_in(
            &env,
            &pred("[minecraft:custom_data~{a:1b}]"),
            &flattened_custom
        ));
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:custom_data~"{a:1b}"]"#),
            &flattened_custom
        ));
        assert!(stack_matches_in(
            &env,
            &pred("[minecraft:custom_data~{}]"),
            &flattened_custom
        ));
        assert!(!stack_matches_in(
            &env,
            &pred("[minecraft:custom_data~{a:1}]"),
            &flattened_custom
        ));

        // MobEffectInstance: show_icon defaults to show_particles; hidden_effect is Details
        // (no id required) and ignored by MobEffectInstance.equals.
        let hidden_fx = stack_with(
            "minecraft:potion",
            1,
            Some(
                r#"{"minecraft:potion_contents":{custom_effects:[{id:"minecraft:speed",duration:200,show_particles:false,hidden_effect:{amplifier:1b,duration:100}}]}}"#,
            ),
        );
        assert!(stack_matches_in(
            &env,
            &pred(
                r#"[minecraft:potion_contents={custom_effects:[{id:"speed",duration:200,show_particles:false,show_icon:false}]}]"#
            ),
            &hidden_fx
        ));
        assert!(!stack_matches_in(
            &env,
            &pred(
                r#"[minecraft:potion_contents={custom_effects:[{id:"speed",duration:200,show_particles:false,show_icon:true}]}]"#
            ),
            &hidden_fx
        ));
        assert!(stack_matches_in(
            &env,
            &pred(
                r#"[minecraft:potion_contents={custom_effects:[{id:"speed",duration:200,show_particles:false,hidden_effect:{amplifier:3b,duration:50}}]}]"#
            ),
            &hidden_fx
        ));

        // Unknown record fields are ignored, while malformed known fields fail equality.
        let cons = stack_with("minecraft:stone", 1, Some(r#"{"minecraft:consumable":{}}"#));
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:consumable={animation:"eat",unknown:1}]"#),
            &cons
        ));
        assert!(!stack_matches_in(
            &env,
            &pred("[minecraft:consumable={animation:1}]"),
            &cons
        ));
        assert!(!stack_matches_in(
            &env,
            &pred("[minecraft:potion_contents={potion:1}]"),
            &potion
        ));

        // Decoded text component equality across string, object, and list forms on
        // written_book_content pages, custom_name, item_name, and lore.
        let book_list_page = stack_with(
            "minecraft:written_book",
            1,
            Some(r#"{"minecraft:written_book_content":{title:"Tales",author:"Alex",pages:[["Page 1"],[{text:"Hi",color:"red",bold:true},"!"]]}}"#),
        );
        assert!(stack_matches_in(
            &env,
            &pred(r##"[minecraft:written_book_content~{pages:{contains:["Page 1",{text:"Hi",color:"#ff5555",bold:1b,extra:["!"]}]}}]"##),
            &book_list_page
        ));
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:written_book_content~{pages:{contains:[["Page 1"]]}}]"#),
            &book_list_page
        ));
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:written_book_content={title:"Tales",author:"Alex",pages:[{type:"text",text:"Page 1",extra:[]},{text:"Hi",color:"red",bold:true,extra:["!"]}]}]"#),
            &book_list_page
        ));
        let named = stack_with(
            "minecraft:stone",
            1,
            Some(
                r#"{"minecraft:custom_name":["Page 1"],"minecraft:item_name":["Blade"],"minecraft:lore":[["Line 1"],[{text:"A",italic:false},"B"]]}"#,
            ),
        );
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:custom_name="Page 1",minecraft:item_name={text:"Blade"},minecraft:lore=["Line 1",{text:"A",italic:0b,extra:["B"]}]]"#),
            &named
        ));

        // Inline armor trim holders (vanilla 26.3) match identically across decoded forms.
        let inline_trim = stack_with(
            "minecraft:diamond_sword",
            1,
            Some(
                r#"{"minecraft:trim":{material:{palette_id:"minecraft:gold",description:"Inline material"},pattern:{asset_id:"minecraft:coast",description:"Inline pattern",decal:0b}}}"#,
            ),
        );
        assert!(stack_matches_in(
            &env,
            &pred(
                r#"[minecraft:trim={material:{palette_id:"gold",description:["Inline material"]},pattern:{asset_id:"coast",description:{text:"Inline pattern"}}}]"#
            ),
            &inline_trim
        ));
        assert!(!stack_matches_in(
            &env,
            &pred(
                r#"[minecraft:trim={material:{palette_id:"gold",description:"Inline material"},pattern:{asset_id:"coast",description:"Inline pattern",decal:true}}]"#
            ),
            &inline_trim
        ));
        let inline_trim_num_decal = stack_with(
            "minecraft:diamond_sword",
            1,
            Some(
                r#"{"minecraft:trim":{material:{palette_id:"minecraft:gold",description:"Inline material"},pattern:{asset_id:"minecraft:coast",description:"Inline pattern",decal:1}}}"#,
            ),
        );
        assert!(stack_matches_in(
            &env,
            &pred(
                r#"[minecraft:trim={material:{palette_id:"gold",description:"Inline material"},pattern:{asset_id:"coast",description:"Inline pattern",decal:true}}]"#
            ),
            &inline_trim_num_decal
        ));
        assert!(!stack_matches_in(
            &env,
            &pred(
                r#"[minecraft:trim={material:{palette_id:"gold",description:"Inline material"},pattern:{asset_id:"coast",description:"Inline pattern",decal:false}}]"#
            ),
            &inline_trim_num_decal
        ));

        // 26.3 compound pot_decorations with ItemStackTemplate sides.
        let pot = stack_with(
            "minecraft:stone",
            1,
            Some(r#"{"minecraft:pot_decorations":{back:{id:"minecraft:brick"}}}"#),
        );
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:pot_decorations={back:{id:"brick",count:1}}]"#),
            &pot
        ));
        assert!(!stack_matches_in(
            &env,
            &pred("[minecraft:pot_decorations={}]"),
            &pot
        ));

        // Firework explosion color byte/long arrays survive stack decode and match `=`.
        let byte_array_star = stack_with(
            "minecraft:firework_star",
            1,
            Some(r#"{"minecraft:firework_explosion":{shape:"star",colors:[B;1b,-1b],fade_colors:[L;2L]}}"#),
        );
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:firework_explosion={shape:"star",colors:[B;1b,-1b],fade_colors:[L;2L]}]"#),
            &byte_array_star
        ));
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:firework_explosion={shape:"star",colors:[1,-1],fade_colors:[2]}]"#),
            &byte_array_star
        ));

        // Firework explosion numeric list colors (`[1.0f]`) and float boolean flags (`has_trail:1.0f`).
        let float_star = stack_with(
            "minecraft:firework_star",
            1,
            Some(
                r#"{"minecraft:firework_explosion":{shape:"star",colors:[1.0f],fade_colors:[2.0d],has_trail:1.0f,has_twinkle:1}}"#,
            ),
        );
        assert!(stack_matches_in(
            &env,
            &pred(
                r#"[minecraft:firework_explosion={shape:"star",colors:[1],fade_colors:[2],has_trail:true,has_twinkle:true}]"#
            ),
            &float_star
        ));
        assert!(stack_matches_in(
            &env,
            &pred(r#"[minecraft:firework_explosion~{shape:"star",has_trail:true,has_twinkle:true}]"#),
            &float_star
        ));
    }
}
