//! `ItemAttributeModifiers.STREAM_CODEC` at the saved-component / wire boundary: the
//! modifiers a stack grants of its own (a command's, a loot function's, a mod's), which
//! `item_attributes` reads in place of its item type's.
use horizon_protocol::{
    buf::{Reader, Writer},
    item::{ATTRIBUTE_OPERATIONS, Component, EQUIPMENT_SLOT_GROUPS},
    registry::Builtin,
};
use {
    horizon_core::{identifier::Identifier, nbt::Tag},
    horizon_world::datafix::dynamic::as_number,
};

/// `ItemAttributeModifiers.Display.Type`'s serialized names, by id.
const DISPLAYS: [&str; 3] = ["default", "hidden", "override"];

/// The stream form as `ItemAttributeModifiers.CODEC` saves it: per entry the attribute
/// (`type`), the modifier's `id`, `amount` and `operation`, then its `slot` and `display`
/// unless they are the defaults (`any`, the default line), which the codec leaves out.
/// An attribute this side has no name for, or bytes the codec would refuse, decode to
/// nothing, and the stack keeps its item type's modifiers.
pub(super) fn decode(bytes: &[u8], builtin: &Builtin) -> Option<Tag> {
    let attributes = builtin.get("minecraft:attribute")?;
    let mut r = Reader::new(bytes);
    let n = r.count(r.remaining()).ok()?;
    let mut entries = Vec::with_capacity(n.min(64));
    for _ in 0..n {
        // Attribute.STREAM_CODEC: the attribute's registry id.
        let attribute = attributes.name(r.varint().ok()?)?;
        // AttributeModifier.STREAM_CODEC: Identifier, double, Operation.
        let id = Identifier::parse(&r.string(32767).ok()?).ok()?;
        let amount = r.f64().ok()?;
        let operation = named(&ATTRIBUTE_OPERATIONS, r.varint().ok()?)?;
        let slot = named(&EQUIPMENT_SLOT_GROUPS, r.varint().ok()?)?;
        let mut entry = vec![
            ("type", Tag::String(attribute.into())),
            ("id", Tag::String(id.to_string().as_str().into())),
            ("amount", Tag::Double(amount.to_bits())),
            ("operation", Tag::String(operation.into())),
        ];
        if slot != EQUIPMENT_SLOT_GROUPS[0] {
            entry.push(("slot", Tag::String(slot.into())));
        }
        // Display.STREAM_CODEC: its type, then an override's text.
        let display = r.varint().ok()?;
        let kind = named(&DISPLAYS, display)?;
        if display != 0 {
            let mut fields = vec![("type", Tag::String(kind.into()))];
            if display == 2 {
                fields.push(("value", r.nbt().ok()?));
            }
            entry.push(("display", Tag::compound(fields)));
        }
        entries.push(Tag::compound(entry));
    }
    (r.remaining() == 0).then_some(Tag::List(entries))
}

/// `table`'s name for network id `id`.
fn named(table: &[&'static str], id: i32) -> Option<&'static str> {
    table.get(usize::try_from(id).ok()?).copied()
}

/// A saved name among `table`'s, as its network id.
fn id_of(table: &[&str], tag: &Tag) -> Option<i32> {
    let Tag::String(name) = tag else {
        return None;
    };
    let name = name.to_string_lossy();
    i32::try_from(table.iter().position(|n| *n == name)?).ok()
}

/// A saved identifier field, as `Identifier.CODEC` reads it: a bare path is `minecraft`'s.
fn identifier(tag: Option<&Tag>) -> Option<Identifier> {
    let Tag::String(text) = tag? else {
        return None;
    };
    Identifier::parse(&text.to_string_lossy()).ok()
}

/// The saved list as `ItemAttributeModifiers.STREAM_CODEC` writes it, each attribute by
/// its registry id from `id`. An entry the codec would refuse (an attribute with no id
/// here, an unknown operation, slot group or display) leaves the component unsent, as
/// one this side can't encode.
pub(crate) fn encode(tag: &Tag, id: &dyn Fn(&str, &str) -> Option<i32>) -> Option<Component> {
    let Tag::List(entries) = tag else {
        return None;
    };
    let mut w = Writer::new();
    w.varint(i32::try_from(entries.len()).ok()?);
    for entry in entries {
        let attribute = id(
            "minecraft:attribute",
            &identifier(entry.get("type"))?.to_string(),
        )?;
        if attribute < 0 {
            return None;
        }
        w.varint(attribute);
        w.string(&identifier(entry.get("id"))?.to_string());
        w.f64(as_number(entry.get("amount")?)?.double_value());
        w.varint(id_of(&ATTRIBUTE_OPERATIONS, entry.get("operation")?)?);
        // `slot` and `display` are optional fields: absent is the default (id 0).
        w.varint(match entry.get("slot") {
            Some(slot) => id_of(&EQUIPMENT_SLOT_GROUPS, slot)?,
            None => 0,
        });
        let display = entry.get("display");
        let kind = match display {
            Some(display) => id_of(&DISPLAYS, display.get("type")?)?,
            None => 0,
        };
        w.varint(kind);
        if let Some(display) = display.filter(|_| kind == 2) {
            let text = display.get("value")?;
            w.bytes(&horizon_core::nbt::write_network(text, Default::default()).ok()?);
        }
    }
    Some(Component::Raw(w.data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_protocol::registry::Ids;

    fn builtin() -> Builtin {
        let names = ["minecraft:armor", "minecraft:luck", "minecraft:scale"];
        Builtin {
            registries: [(
                "minecraft:attribute".to_owned(),
                Ids::from_names(names.map(str::to_owned).to_vec()),
            )]
            .into(),
        }
    }

    fn registry_id(registry: &str, name: &str) -> Option<i32> {
        builtin().get(registry)?.id(name)
    }

    /// One entry as vanilla's stream codec writes it.
    fn entry(w: &mut Writer, attribute: i32, id: &str, amount: f64, operation: i32, slot: i32) {
        w.varint(attribute);
        w.string(id);
        w.f64(amount);
        w.varint(operation);
        w.varint(slot);
    }

    /// Every field the stream carries reads into the saved list, with the codec's defaults
    /// (slot `any`, the default display) left out, and writes back to the same bytes.
    #[test]
    fn a_servers_modifiers_read_as_the_saved_list_and_back() {
        let mut w = Writer::new();
        w.varint(3);
        entry(&mut w, 0, "minecraft:armor.chestplate", 6.0, 0, 6);
        w.varint(0);
        entry(&mut w, 1, "test:charm", 0.25, 2, 0);
        w.varint(1);
        entry(&mut w, 2, "test:tall", -0.5, 1, 8);
        w.varint(2);
        w.nbt(&Tag::String("Taller".into()));
        let tag = decode(&w.data, &builtin()).unwrap();
        let text = |s: &str| Tag::String(s.into());
        assert_eq!(
            tag,
            Tag::List(vec![
                Tag::compound([
                    ("type", text("minecraft:armor")),
                    ("id", text("minecraft:armor.chestplate")),
                    ("amount", Tag::Double(6.0f64.to_bits())),
                    ("operation", text("add_value")),
                    ("slot", text("chest")),
                ]),
                Tag::compound([
                    ("type", text("minecraft:luck")),
                    ("id", text("test:charm")),
                    ("amount", Tag::Double(0.25f64.to_bits())),
                    ("operation", text("add_multiplied_total")),
                    ("display", Tag::compound([("type", text("hidden"))])),
                ]),
                Tag::compound([
                    ("type", text("minecraft:scale")),
                    ("id", text("test:tall")),
                    ("amount", Tag::Double((-0.5f64).to_bits())),
                    ("operation", text("add_multiplied_base")),
                    ("slot", text("armor")),
                    (
                        "display",
                        Tag::compound([("type", text("override")), ("value", text("Taller"))]),
                    ),
                ]),
            ])
        );
        assert_eq!(encode(&tag, &registry_id), Some(Component::Raw(w.data)));
    }

    /// A saved list as a command writes it, bare ids and an int amount, goes out in the
    /// codec's own form.
    #[test]
    fn a_saved_list_is_sent_in_the_codecs_form() {
        let text = |s: &str| Tag::String(s.into());
        let saved = Tag::List(vec![Tag::compound([
            ("type", text("armor")),
            ("id", text("plate")),
            ("amount", Tag::Int(3)),
            ("operation", text("add_value")),
            ("display", Tag::compound([("type", text("default"))])),
        ])]);
        let mut w = Writer::new();
        w.varint(1);
        entry(&mut w, 0, "minecraft:plate", 3.0, 0, 0);
        w.varint(0);
        assert_eq!(encode(&saved, &registry_id), Some(Component::Raw(w.data)));
    }

    /// What the codec refuses decodes to nothing, so the stack keeps its item type's
    /// modifiers; and a list naming what this side can't send is not sent at all.
    #[test]
    fn what_the_codec_refuses_is_neither_read_nor_sent() {
        let one = |attribute: i32, operation: i32, slot: i32, display: i32| {
            let mut w = Writer::new();
            w.varint(1);
            entry(&mut w, attribute, "test:x", 1.0, operation, slot);
            w.varint(display);
            w.data
        };
        for bytes in [
            one(3, 0, 0, 0),
            one(0, 3, 0, 0),
            one(0, 0, 11, 0),
            one(0, 0, 0, 3),
        ] {
            assert_eq!(decode(&bytes, &builtin()), None);
        }
        let mut trailing = one(0, 0, 0, 0);
        trailing.push(0);
        assert_eq!(decode(&trailing, &builtin()), None);
        assert_eq!(decode(&one(0, 0, 0, 0), &Builtin::default()), None);

        let text = |s: &str| Tag::String(s.into());
        let saved = |attribute: &str, operation: &str, slot: &str| {
            Tag::List(vec![Tag::compound([
                ("type", text(attribute)),
                ("id", text("test:x")),
                ("amount", Tag::Double(1.0f64.to_bits())),
                ("operation", text(operation)),
                ("slot", text(slot)),
            ])])
        };
        for list in [
            saved("minecraft:gravity", "add_value", "any"),
            saved("minecraft:armor", "multiply", "any"),
            saved("minecraft:armor", "add_value", "pocket"),
            Tag::compound([("modifiers", Tag::List(Vec::new()))]),
        ] {
            assert_eq!(encode(&list, &registry_id), None);
        }
    }
}
