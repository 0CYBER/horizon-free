//! What a converted mob carries over that lives outside `Mob`: its tags, its team and its mods'
//! attachments (`Mob.convertTo`).
use super::*;

impl Game<'_> {
    /// Transfer the entity-owned state that is stored outside `Mob`.
    pub(super) fn transfer_conversion_identity(&mut self, from: u32, to: u32) {
        use crate::game::commands::Target;
        use horizon_client::save::uuid_text;

        let tags = self.tags_of(Target::Mob(from));
        if !tags.is_empty() {
            self.set_entity_tags(Target::Mob(to), &tags);
        }
        self.load_scoreboard();
        let old_name = uuid_text(self.uuid_for(from));
        let team = self
            .score
            .board
            .players_team(&old_name)
            .map(|t| t.name.clone());
        if let Some(team) = team {
            let new_name = uuid_text(self.uuid_for(to));
            self.score.board.add_to_team(&new_name, &team);
            self.score.board.remove_from_team(&old_name);
        }
    }
    /// `IEntityExtension.copyAttachmentsFrom(from, false)` (NeoForge's conversion copy): every
    /// attachment of the old mob with a codec, a mod's that isn't loaded included, goes to the
    /// new mob of type `kind` through its saved form, over what the new mob's construction
    /// gave it. A value without a codec stays behind. The copy is not synced on its own: the new
    /// mob's spawn carries its synced values. A mob with no attachments costs one lookup.
    pub(super) fn copy_conversion_attachments(&mut self, from: u32, to: u32, kind: &str) {
        let Some(saved) = self.entity_attachments.get(&from).map(|map| map.to_nbt()) else {
            return;
        };
        if saved.is_empty() {
            return;
        }
        // The new mob is constructed (`EntityConstructing`) before the copy, as `convertTo`
        // builds it before `copyAttachmentsFrom`, so the copy wins key by key.
        self.initialize_entity(mod_api::Holder::Entity(to), kind);
        self.entity_attachments
            .entry(to)
            .or_default()
            .read_saved_over(saved);
    }
}

#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::with_game;
    use horizon_client::mobs::{Mob, conversion::Params};
    use horizon_core::{geometry::Vec3, nbt::Tag};
    use horizon_mod_runtime::attachments::{AttachmentCodec, AttachmentError};
    use mod_api::{AttachmentType, Holder};

    static MARK: AttachmentType<i32> = AttachmentType {
        namespace: "conversion_test",
        name: "mark",
        default: || 0,
        codec: Some(AttachmentCodec {
            encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
            decode: |tag| match tag.get(&"n".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            },
        }),
        copy_on_death: false,
        sync: false,
    };
    static LOOSE: AttachmentType<i32> = AttachmentType {
        namespace: "conversion_test",
        name: "loose",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };

    /// A villager struck into a witch keeps the attachment that has a codec and leaves the
    /// one without behind (`copyAttachmentsFrom(villager, false)`).
    #[test]
    #[ignore = "daily"]
    fn a_converted_mob_keeps_its_serializable_attachments() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let kind = game
                .res
                .mob_types()
                .get("minecraft:villager")
                .unwrap()
                .clone();
            let villager = Mob::new(500, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            game.mobs.push(villager.clone());
            MARK.set(game, Holder::Entity(500), 7).unwrap();
            LOOSE.set(game, Holder::Entity(500), 3).unwrap();
            let params = Params {
                keep_equipment: false,
                preserve_can_pick_up_loot: false,
            };
            let witch = game
                .convert_to(&villager, "minecraft:witch", params, false, |_, _| {})
                .unwrap();
            assert_eq!(MARK.get_existing(game, Holder::Entity(witch)), Ok(Some(7)));
            assert_eq!(LOOSE.get_existing(game, Holder::Entity(witch)), Ok(None));
        });
    }
}
