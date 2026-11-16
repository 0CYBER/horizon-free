use super::EntityType;
use std::collections::BTreeMap;

/// Preserve template number text while adding content attributes and sound selections.
pub(super) fn mod_entity_row(
    row: &serde_json::value::RawValue,
    t: &EntityType,
) -> std::result::Result<Vec<u8>, String> {
    use serde_json::value::{RawValue, to_raw_value};
    type Raw = BTreeMap<String, Box<RawValue>>;
    let fail = |e: serde_json::Error| format!("{}: {e}", t.template);
    let mut fields: Raw = serde_json::from_str(row.get()).map_err(fail)?;
    if !t.attributes.is_empty() {
        let mut attributes: Raw = fields.get("attributes").map_or(Ok(Raw::new()), |a| {
            serde_json::from_str(a.get()).map_err(fail)
        })?;
        for (id, value) in &t.attributes {
            // Refused rather than added: the mob reads the template's class attributes.
            let Some(base) = attributes.get_mut(id) else {
                return Err(format!(
                    "{}: attribute {id} is not one of {}'s",
                    t.id, t.template
                ));
            };
            *base = to_raw_value(value).map_err(fail)?;
        }
        fields.insert(
            "attributes".into(),
            to_raw_value(&attributes).map_err(fail)?,
        );
    }
    if !t.passenger_attachments.is_empty() {
        let raw = to_raw_value(&t.passenger_attachments).map_err(fail)?;
        fields.insert("passenger_attachments".into(), raw);
    }
    for (key, value) in [
        ("shadow_radius", t.shadow_radius),
        ("shadow_strength", t.shadow_strength),
    ] {
        if let Some(value) = value {
            fields.insert(key.into(), to_raw_value(&value).map_err(fail)?);
        }
    }
    fields.insert("template".into(), to_raw_value(&t.template).map_err(fail)?);
    if t.sounds != Default::default() {
        fields.insert("sounds".into(), to_raw_value(&t.sounds).map_err(fail)?);
    }
    serde_json::to_vec(&fields).map_err(fail)
}

#[cfg(test)]
#[path = "entity_sounds_tests.rs"]
mod tests;
