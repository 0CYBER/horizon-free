//! LockCode's ItemPredicate and the component predicates it stores.
use super::{book_page, decode_component_entries, stored_components, string, text};
use crate::{component_codec::nbt_bool, identifier::normalize_id};
use horizon_core::nbt::Tag;
use horizon_world::datafix::dynamic::{Number, as_number};

pub(super) fn decode(
    tag: &Tag,
    known_holder: &dyn Fn(&str, &str) -> bool,
    component_types: &[String],
    valid_nbt_source: &dyn Fn(&str, &str) -> bool,
) -> Option<Tag> {
    Decoder {
        known_holder,
        component_types,
        valid_nbt_source,
    }
    .item(tag)
}

struct Decoder<'a> {
    known_holder: &'a dyn Fn(&str, &str) -> bool,
    component_types: &'a [String],
    valid_nbt_source: &'a dyn Fn(&str, &str) -> bool,
}

impl Decoder<'_> {
    fn item(&self, tag: &Tag) -> Option<Tag> {
        let mut components = stored_components(
            tag.get("components"),
            self.known_holder,
            self.component_types,
            self.valid_nbt_source,
        );
        components.retain(|id, _| {
            (self.known_holder)("minecraft:data_component_type", &id.to_string_lossy())
        });
        let predicates = decode_component_entries(tag.get("predicates"), |id, value| {
            self.subpredicate(id, value)
        });
        record(
            tag,
            [
                ("items", self.holders(tag.get("items"), "minecraft:item")),
                ("count", tag.get("count").and_then(bounded_int_range)),
                (
                    "components",
                    (!components.is_empty()).then_some(Tag::Compound(components)),
                ),
                (
                    "predicates",
                    (!predicates.is_empty()).then_some(Tag::Compound(predicates)),
                ),
            ],
        )
    }

    /// DataComponentPredicate resolves concrete types before falling back to AnyValue.
    fn subpredicate(&self, id: &str, tag: &Tag) -> Option<Tag> {
        let range = |key| tag.get(key).and_then(bounded_int_range);
        let collection =
            |key, decode: &dyn Fn(&Tag) -> Option<Tag>| collection_predicate(tag.get(key)?, decode);
        match id {
            "minecraft:custom_data" => custom_data(tag),
            "minecraft:damage" => record(
                tag,
                [
                    ("durability", range("durability")),
                    ("damage", range("damage")),
                ],
            ),
            "minecraft:enchantments" | "minecraft:stored_enchantments" => list(tag, |entry| {
                record(
                    entry,
                    [
                        (
                            "enchantments",
                            self.holders(entry.get("enchantments"), "minecraft:enchantment"),
                        ),
                        ("levels", entry.get("levels").and_then(bounded_int_range)),
                    ],
                )
            })
            .map(Tag::List),
            "minecraft:villager/variant" => self.holders(Some(tag), "minecraft:villager_type"),
            "minecraft:potion_contents" => record(
                tag,
                [
                    (
                        "potions",
                        self.holders(tag.get("potions"), "minecraft:potion"),
                    ),
                    (
                        "effects",
                        collection("effects", &|value| self.effects(value)),
                    ),
                ],
            ),
            "minecraft:container" | "minecraft:bundle_contents" => record(
                tag,
                [("items", collection("items", &|value| self.item(value)))],
            ),
            "minecraft:firework_explosion" => firework(tag),
            "minecraft:fireworks" => record(
                tag,
                [
                    ("explosions", collection("explosions", &firework)),
                    ("flight_duration", range("flight_duration")),
                ],
            ),
            "minecraft:writable_book_content" => {
                record(tag, [("pages", collection("pages", &string_tag))])
            }
            "minecraft:written_book_content" => record(
                tag,
                [
                    (
                        "pages",
                        collection("pages", &|page| {
                            book_page::decode(page, self.valid_nbt_source)
                        }),
                    ),
                    ("author", tag.get("author").and_then(string_tag)),
                    ("title", tag.get("title").and_then(string_tag)),
                    ("generation", range("generation")),
                    ("resolved", tag.get("resolved").and_then(bool_tag)),
                ],
            ),
            "minecraft:attribute_modifiers" => record(
                tag,
                [(
                    "modifiers",
                    collection("modifiers", &|value| self.modifier(value)),
                )],
            ),
            "minecraft:trim" => record(
                tag,
                [
                    (
                        "material",
                        self.holders(tag.get("material"), "minecraft:trim_material"),
                    ),
                    (
                        "pattern",
                        self.holders(tag.get("pattern"), "minecraft:trim_pattern"),
                    ),
                ],
            ),
            "minecraft:jukebox_playable" => record(
                tag,
                [(
                    "song",
                    self.holders(tag.get("song"), "minecraft:jukebox_song"),
                )],
            ),
            _ if self.component_types.iter().any(|component| component == id) => record(tag, []),
            // Canonicalizing an already loaded tag has no registry context.
            _ if self.component_types.is_empty() => Some(tag.clone()),
            _ => None,
        }
    }

    fn modifier(&self, tag: &Tag) -> Option<Tag> {
        record(
            tag,
            [
                (
                    "attribute",
                    self.holders(tag.get("attribute"), "minecraft:attribute"),
                ),
                ("id", tag.get("id").and_then(identifier)),
                (
                    "amount",
                    tag.get("amount")
                        .and_then(double_range)
                        .and_then(nonempty_range),
                ),
                (
                    "operation",
                    enum_tag(
                        tag.get("operation"),
                        &["add_value", "add_multiplied_base", "add_multiplied_total"],
                    ),
                ),
                (
                    "slot",
                    enum_tag(
                        tag.get("slot"),
                        &[
                            "any", "mainhand", "offhand", "hand", "feet", "legs", "chest", "head",
                            "armor", "body", "saddle",
                        ],
                    ),
                ),
            ],
        )
    }

    fn effects(&self, tag: &Tag) -> Option<Tag> {
        let Tag::Compound(_) = tag else { return None };
        Some(Tag::Compound(decode_component_entries(
            Some(tag),
            |id, effect| {
                if !(self.known_holder)("minecraft:mob_effect", id) {
                    return None;
                }
                record(
                    effect,
                    [
                        (
                            "amplifier",
                            effect.get("amplifier").and_then(bounded_int_range),
                        ),
                        (
                            "duration",
                            effect.get("duration").and_then(bounded_int_range),
                        ),
                        ("ambient", effect.get("ambient").and_then(bool_tag)),
                        ("visible", effect.get("visible").and_then(bool_tag)),
                    ],
                )
            },
        )))
    }

    /// RegistryCodecs.holderSet writes a singleton as an identifier.
    fn holders(&self, tag: Option<&Tag>, registry: &str) -> Option<Tag> {
        let direct = |tag: &Tag| {
            let id = normalize_id(&text(Some(tag))?)?;
            (self.known_holder)(registry, &id).then(|| string(&id))
        };
        match tag? {
            tag @ Tag::String(value) => {
                let value = value.to_string_lossy();
                if let Some(name) = value.strip_prefix('#') {
                    let tag = format!("#{}", normalize_id(name)?);
                    (self.known_holder)(registry, &tag).then(|| string(&tag))
                } else {
                    direct(tag)
                }
            }
            tag => {
                let mut entries = list(tag, direct)?;
                if entries.len() == 1 {
                    entries.pop()
                } else {
                    Some(Tag::List(entries))
                }
            }
        }
    }
}

/// Record codecs expose partial records: each optional field may decode independently.
fn record(tag: &Tag, fields: impl IntoIterator<Item = (&'static str, Option<Tag>)>) -> Option<Tag> {
    matches!(tag, Tag::Compound(_)).then(|| {
        Tag::compound(
            fields
                .into_iter()
                .filter_map(|(key, value)| Some((key, value?))),
        )
    })
}

fn list(tag: &Tag, decode: impl FnMut(&Tag) -> Option<Tag>) -> Option<Vec<Tag>> {
    match tag {
        Tag::List(values) => Some(values.iter().filter_map(decode).collect()),
        Tag::ByteArray(_) | Tag::IntArray(_) | Tag::LongArray(_) => Some(Vec::new()),
        _ => None,
    }
}

fn collection_predicate(tag: &Tag, element: &dyn Fn(&Tag) -> Option<Tag>) -> Option<Tag> {
    record(
        tag,
        [
            (
                "contains",
                tag.get("contains")
                    .and_then(|values| list(values, element))
                    .map(Tag::List),
            ),
            (
                "count",
                tag.get("count")
                    .and_then(|values| {
                        list(values, |entry| {
                            // Both fields are required on a CollectionCountsPredicate entry.
                            Some(Tag::compound([
                                ("test", element(entry.get("test")?)?),
                                ("count", int_range(entry.get("count")?)?),
                            ]))
                        })
                    })
                    .map(Tag::List),
            ),
            ("size", tag.get("size").and_then(int_range)),
        ],
    )
}

fn firework(tag: &Tag) -> Option<Tag> {
    record(
        tag,
        [
            (
                "shape",
                enum_tag(
                    tag.get("shape"),
                    &["small_ball", "large_ball", "star", "creeper", "burst"],
                ),
            ),
            ("has_twinkle", tag.get("has_twinkle").and_then(bool_tag)),
            ("has_trail", tag.get("has_trail").and_then(bool_tag)),
        ],
    )
}

fn string_tag(tag: &Tag) -> Option<Tag> {
    matches!(tag, Tag::String(_)).then(|| tag.clone())
}

fn enum_tag(tag: Option<&Tag>, values: &[&str]) -> Option<Tag> {
    let tag = tag?;
    values
        .contains(&text(Some(tag))?.as_str())
        .then(|| tag.clone())
}

fn bool_tag(tag: &Tag) -> Option<Tag> {
    nbt_bool(tag).map(super::boolean)
}

fn identifier(tag: &Tag) -> Option<Tag> {
    normalize_id(&text(Some(tag))?).map(|id| string(&id))
}

fn int_range(tag: &Tag) -> Option<Tag> {
    bounds(tag, Number::int_value, Tag::Int)
}

fn bounded_int_range(tag: &Tag) -> Option<Tag> {
    int_range(tag).and_then(nonempty_range)
}

fn double_range(tag: &Tag) -> Option<Tag> {
    bounds(tag, Number::double_value, |n| Tag::Double(n.to_bits()))
}

/// MinMaxBounds writes equal endpoints as one number and an unbounded range as {}.
fn bounds<T: PartialOrd>(
    tag: &Tag,
    number: impl Fn(Number) -> T,
    encode: impl Fn(T) -> Tag,
) -> Option<Tag> {
    let read = |tag: &Tag| as_number(tag).map(&number);
    if !matches!(tag, Tag::Compound(_)) {
        return read(tag).map(encode);
    }
    let min = tag.get("min").and_then(read);
    let max = tag.get("max").and_then(read);
    match (min, max) {
        (Some(min), Some(max)) if min == max => Some(encode(min)),
        (Some(min), Some(max)) if min > max => None,
        (min, max) => record(tag, [("min", min.map(&encode)), ("max", max.map(encode))]),
    }
}

fn nonempty_range(tag: Tag) -> Option<Tag> {
    (!matches!(&tag, Tag::Compound(fields) if fields.is_empty())).then_some(tag)
}

/// NbtPredicate uses TagParser.LENIENT_CODEC, which writes compact SNBT.
fn custom_data(tag: &Tag) -> Option<Tag> {
    let parsed;
    let compound = match tag {
        Tag::Compound(_) => tag,
        Tag::String(s) => {
            let (value, end) = horizon_core::nbt::parse_snbt_utf16(&s.0).ok()?;
            if !matches!(value, Tag::Compound(_))
                || !s.0[end..]
                    .iter()
                    .all(|&unit| horizon_core::nbt::snbt_whitespace(u32::from(unit)))
            {
                return None;
            }
            parsed = value;
            &parsed
        }
        _ => return None,
    };
    Some(string(&horizon_world::snbt_text::to_snbt(compound)))
}
