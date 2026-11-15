//! A mod's AI pause (`MobRef::set_ai_paused`), saved with the mob as an
//! attachment-style value: `horizon:attachments` / `horizon` / `ai_paused`,
//! never a vanilla key and never `NoAI`. Only a paused mob writes it.
use horizon_core::nbt::{Compound, Tag};

/// The entity's attachment compound (`horizon_mod_runtime::attachments::ATTACHMENTS_KEY`).
const ATTACHMENTS: &str = "horizon:attachments";
const NAMESPACE: &str = "horizon";
const NAME: &str = "ai_paused";

/// Put the pause into `tag`, or take it out (and any compound it leaves
/// empty) for a mob that is not paused. Called after the mods' attachments
/// are written, so it decides the value either way.
pub fn write_ai_paused(paused: bool, tag: &mut Tag) {
    let Tag::Compound(root) = tag else { return };
    if paused {
        let attachments = root
            .entry(ATTACHMENTS.into())
            .or_insert_with(|| Tag::Compound(Compound::new()));
        // A saved value that is not a compound is kept as it was, as the mods' map keeps it.
        let Tag::Compound(attachments) = attachments else {
            return;
        };
        let names = attachments
            .entry(NAMESPACE.into())
            .or_insert_with(|| Tag::Compound(Compound::new()));
        if let Tag::Compound(names) = names {
            names.insert(NAME.into(), Tag::Byte(1));
        }
        return;
    }
    let Some(Tag::Compound(attachments)) = root.get_mut(&ATTACHMENTS.into()) else {
        return;
    };
    let Some(Tag::Compound(names)) = attachments.get_mut(&NAMESPACE.into()) else {
        return;
    };
    if names.remove(&NAME.into()).is_none() {
        return;
    }
    if names.is_empty() {
        attachments.remove(&NAMESPACE.into());
    }
    if attachments.is_empty() {
        root.remove(&ATTACHMENTS.into());
    }
}

/// Whether `tag` saved the mob paused.
pub(super) fn read(tag: &Tag) -> bool {
    tag.get(ATTACHMENTS)
        .and_then(|a| a.get(NAMESPACE))
        .is_some_and(|names| super::flag(names, NAME))
}

#[cfg(test)]
mod tests {
    use super::{read, write_ai_paused};
    use horizon_core::nbt::{Compound, Tag};

    /// The pause goes in under Horizon's own namespace beside a mod's
    /// attachment, comes back out on a resume with nothing left behind, and
    /// never touches `NoAI`.
    #[test]
    fn the_pause_is_an_attachment_value_and_never_no_ai() {
        let mods = Compound::from_iter([("example".into(), Tag::Compound(Compound::new()))]);
        let mut tag = Tag::compound([("horizon:attachments", Tag::Compound(mods.clone()))]);
        assert!(!read(&tag));
        write_ai_paused(true, &mut tag);
        assert!(read(&tag));
        assert!(tag.get("NoAI").is_none());
        let saved = tag.get("horizon:attachments").unwrap();
        assert_eq!(
            saved.get("horizon").unwrap().get("ai_paused"),
            Some(&Tag::Byte(1))
        );
        assert!(
            saved.get("example").is_some(),
            "the mod's attachment is kept"
        );
        write_ai_paused(false, &mut tag);
        assert!(!read(&tag));
        assert_eq!(tag.get("horizon:attachments"), Some(&Tag::Compound(mods)));

        let mut bare = Tag::compound([("id", Tag::String("minecraft:pig".into()))]);
        let before = bare.clone();
        write_ai_paused(false, &mut bare);
        assert_eq!(bare, before, "an unpaused mob writes nothing");
        write_ai_paused(true, &mut bare);
        write_ai_paused(false, &mut bare);
        assert_eq!(bare, before, "and a resume leaves nothing behind");
    }
}
