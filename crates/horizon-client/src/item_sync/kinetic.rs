//! `KineticWeapon.STREAM_CODEC` at the saved-component / wire boundary.
use horizon_protocol::{
    buf::{Reader, Writer},
    item::Component,
    registry::Builtin,
};
use {
    horizon_core::{identifier::Identifier, nbt::Tag},
    horizon_world::datafix::dynamic::as_number,
};

const CONDITIONS: [&str; 3] = [
    "dismount_conditions",
    "knockback_conditions",
    "damage_conditions",
];

pub(super) fn decode(bytes: &[u8], builtin: &Builtin) -> Option<Tag> {
    let mut r = Reader::new(bytes);
    let mut fields = vec![
        ("contact_cooldown_ticks", Tag::Int(r.varint().ok()?)),
        ("delay_ticks", Tag::Int(r.varint().ok()?)),
    ];
    for key in CONDITIONS {
        if r.bool().ok()? {
            fields.push((
                key,
                Tag::compound([
                    ("max_duration_ticks", Tag::Int(r.varint().ok()?)),
                    ("min_speed", Tag::Float(r.f32().ok()?.to_bits())),
                    ("min_relative_speed", Tag::Float(r.f32().ok()?.to_bits())),
                ]),
            ));
        }
    }
    fields.push(("forward_movement", Tag::Float(r.f32().ok()?.to_bits())));
    fields.push(("damage_multiplier", Tag::Float(r.f32().ok()?.to_bits())));
    for key in ["sound", "hit_sound"] {
        if r.bool().ok()? {
            let id = r.varint().ok()?;
            if id < 0 {
                return None;
            }
            let sound = if id == 0 {
                let mut sound = vec![(
                    "sound_id",
                    Tag::String(r.string(32767).ok()?.as_str().into()),
                )];
                if r.bool().ok()? {
                    sound.push(("range", Tag::Float(r.f32().ok()?.to_bits())));
                }
                Tag::compound(sound)
            } else {
                Tag::String(builtin.get("minecraft:sound_event")?.name(id - 1)?.into())
            };
            fields.push((key, sound));
        }
    }
    (r.remaining() == 0).then(|| Tag::compound(fields))
}

fn integer(tag: Option<&Tag>, default: i32) -> Option<i32> {
    tag.map_or(Some(default), |t| Some(as_number(t)?.int_value()))
}
fn float(tag: Option<&Tag>, default: f32) -> Option<f32> {
    tag.map_or(Some(default), |t| Some(as_number(t)?.float_value()))
}

pub(crate) fn encode(tag: &Tag, id: &dyn Fn(&str, &str) -> Option<i32>) -> Option<Component> {
    let Tag::Compound(_) = tag else { return None };
    let mut w = Writer::new();
    w.varint(integer(tag.get("contact_cooldown_ticks"), 10)?);
    w.varint(integer(tag.get("delay_ticks"), 0)?);
    for key in CONDITIONS {
        let condition = tag.get(key);
        w.bool(condition.is_some());
        if let Some(condition) = condition {
            w.varint(as_number(condition.get("max_duration_ticks")?)?.int_value());
            w.f32(float(condition.get("min_speed"), 0.0)?);
            w.f32(float(condition.get("min_relative_speed"), 0.0)?);
        }
    }
    w.f32(float(tag.get("forward_movement"), 0.0)?);
    w.f32(float(tag.get("damage_multiplier"), 1.0)?);
    for key in ["sound", "hit_sound"] {
        let sound = tag.get(key);
        w.bool(sound.is_some());
        match sound {
            Some(Tag::String(name)) => {
                let name = Identifier::parse(&name.to_string_lossy()).ok()?;
                w.varint(id("minecraft:sound_event", &name.to_string())? + 1)
            }
            Some(sound) => {
                let Tag::String(name) = sound.get("sound_id")? else {
                    return None;
                };
                let name = Identifier::parse(&name.to_string_lossy()).ok()?;
                w.varint(0);
                w.string(&name.to_string());
                // SoundEvent.DIRECT_CODEC uses lenientOptionalFieldOf for range.
                let range = sound.get("range").and_then(as_number);
                w.bool(range.is_some());
                if let Some(range) = range {
                    w.f32(range.float_value());
                }
            }
            None => {}
        }
    }
    Some(Component::Raw(w.data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_sound_holders_have_no_saved_projection() {
        for id in [-1, i32::MIN] {
            let mut w = Writer::new();
            // Default kinetic fields followed by a present sound holder.
            w.data
                .extend([10, 0, 0, 0, 0, 0, 0, 0, 0, 63, 128, 0, 0, 1]);
            w.varint(id);
            w.bool(false);
            assert!(decode(&w.data, &Builtin::default()).is_none());
        }
    }
}
