//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

impl RegistrationRefusalRegistrar for PartRegistrar {
    fn registration_refusal(&mut self, reason: String) {
        if self.self_refusal.is_none() {
            let context = crate::RegistrationContext::new();
            context.refuse(&reason);
            self.self_refusal = mod_vocab::__private::take_registration_refusal(&context);
        }
    }
    fn registration_warning(&mut self, warning: crate::LoadingMessage) {
        let kept = self
            .warnings
            .iter()
            .filter(|(owner, _)| owner == self.mod_id)
            .count();
        if kept < mod_vocab::__private::MAX_LOADING_WARNINGS {
            // A guest's message crosses the wire whole: cut its key and arguments again here.
            let args: Vec<&str> = warning.args().iter().map(String::as_str).collect();
            let warning = crate::LoadingMessage::new(warning.key(), &args);
            self.warnings.push((self.mod_id.to_owned(), warning));
        }
    }
}

impl PartRegistrar {
    pub(crate) fn registration_checkpoint(&self) -> impl FnOnce(&mut Self) + use<> {
        let parts = self.parts.clone();
        let tags = self.tags.clone();
        let keys = self.registry_keys.borrow().clone();
        let menu_recipe_books = self.menu_recipe_books.clone();
        let recipe_book_types = self.recipe_book_types.clone();
        let registered = self.registered;
        let entity_selector_bytes = self.entity_selector_bytes;
        let block_entity_transforms = self.block_entity_transforms.clone();
        // Append-only tables retain previous mods' entries without requiring
        // Clone on runtime callback objects or their eventual frozen caches.
        macro_rules! checkpoint {
            ($($field:ident),* $(,)?) => {{
                $(let $field = self.$field.len();)*
                move |registrar: &mut Self| {
                    $(registrar.$field.truncate($field);)*
                    registrar.block_entity_transforms = block_entity_transforms;
                    registrar.parts = parts;
                    registrar.tags = tags;
                    *registrar.registry_keys.borrow_mut() = keys;
                    registrar.menu_recipe_books = menu_recipe_books;
                    registrar.recipe_book_types = recipe_book_types;
                    registrar.registered = registered;
                    registrar.entity_selector_bytes = entity_selector_bytes;
                    registrar.self_refusal = None;
                }
            }};
        }
        checkpoint!(
            game_rule_categories,
            game_rules,
            custom_stats,
            block_entity_types,
            entity_attributes,
            general_entity_types,
            world_fluids,
            entity_attribute_modifications,
            attributes,
            spawn_placements,
            entity_part_states,
            biome_regions,
            controllers,
            arm_poses,
            structure_types,
            dimensions,
            chunk_generators,
            biome_sources,
            density_function_types,
            refused
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::{Hooks, ModEntry, Registrar, RegistrationContext};

    struct Callback;
    impl crate::BlockPart for Callback {
        fn block_subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
    }
    impl crate::BlockEntityPart for Callback {
        fn block_entity_subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
    }

    fn region() -> crate::BiomeRegion {
        crate::BiomeRegion::new(
            "minecraft:overworld".into(),
            5,
            vec![crate::BiomePoint {
                biome: "minecraft:plains".into(),
                temperature: (0.0, 0.0),
                humidity: (0.0, 0.0),
                continentalness: (0.0, 0.0),
                erosion: (0.0, 0.0),
                depth: (0.0, 0.0),
                weirdness: (0.0, 0.0),
                offset: 0.0,
            }],
        )
    }

    struct Refuses;
    impl Hooks for Refuses {
        fn subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
        fn register(&self, r: &mut dyn Registrar, cx: &RegistrationContext) {
            r.declare_fluid("before");
            r.declare_biome_region("rejected", region()).unwrap();
            let kind = r
                .block_entity_type("callback")
                .blocks(&[])
                .transform(|data: &crate::Compound, _: crate::StructureTransform| Ok(data.clone()))
                .build();
            r.block_entity_part(&kind, Callback).unwrap();
            cx.refuse("bootstrap failed: café\nmissing bridge");
            cx.refuse("ignored");
            r.declare_fluid("foreign:after");
        }
    }
    struct Healthy;
    impl Hooks for Healthy {
        fn subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.declare_fluid("soup");
            r.declare_biome_region("kept", region()).unwrap();
        }
    }

    struct Warns;
    impl Hooks for Warns {
        fn subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
        fn register(&self, r: &mut dyn Registrar, cx: &RegistrationContext) {
            cx.warn("test.warn.optional", &["jei"]);
            r.declare_fluid("soup");
        }
    }

    #[test]
    fn a_warning_keeps_the_mod_loading_and_is_published_for_it() {
        let mods = [ModEntry::new("warns", Box::new(Warns))];
        let registrar = super::super::register_all(&mods, |_| false);
        assert!(registrar.refused.is_empty());
        assert!(
            registrar
                .registry_keys
                .borrow()
                .declared_fluids
                .contains_key("warns:soup")
        );
        assert_eq!(
            registrar.warnings,
            [(
                "warns".to_owned(),
                crate::LoadingMessage::new("test.warn.optional", &["jei"])
            )]
        );
    }

    struct WarnsPastTheCap;
    impl Hooks for WarnsPastTheCap {
        fn subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
        fn register(&self, _: &mut dyn Registrar, cx: &RegistrationContext) {
            for i in 0..=mod_vocab::__private::MAX_LOADING_WARNINGS {
                cx.warn("test.warn.many", &[&i.to_string()]);
            }
        }
    }

    #[test]
    fn warnings_are_published_under_the_mod_that_gave_them() {
        let mods = [
            ModEntry::new("warns", Box::new(Warns)),
            ModEntry::new("quiet", Box::new(Healthy)),
            ModEntry::new("many", Box::new(WarnsPastTheCap)),
        ];
        crate::__private::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false)
            .unwrap();
        assert_eq!(
            crate::__private::registration_warnings("warns"),
            [crate::LoadingMessage::new("test.warn.optional", &["jei"])]
        );
        assert!(crate::__private::registration_warnings("quiet").is_empty());
        let many = crate::__private::registration_warnings("many");
        assert_eq!(many.len(), mod_vocab::__private::MAX_LOADING_WARNINGS);
        assert_eq!(many[15].args(), ["15"]);
    }

    #[test]
    fn the_host_caps_and_cuts_what_a_guest_sends() {
        use crate::RegistrationRefusalRegistrar;
        let mut registrar = super::PartRegistrar::new("guest");
        use crate::wire::{Reader, Wire, Writer};
        let long = format!("{}é", "a".repeat(4095));
        let args = vec![long.clone(); 20];
        let mut wire = Writer::new();
        long.put(&mut wire);
        args.put(&mut wire);
        let bytes = wire.into_bytes();
        for _ in 0..=mod_vocab::__private::MAX_LOADING_WARNINGS {
            // As the guest dispatch arm delivers it: the owner is the registrar's mod, not the message's.
            let message = crate::LoadingMessage::take(&mut Reader::new(&bytes)).unwrap();
            registrar.registration_warning(message);
        }
        assert_eq!(
            registrar.warnings.len(),
            mod_vocab::__private::MAX_LOADING_WARNINGS
        );
        assert!(
            registrar
                .warnings
                .iter()
                .all(|(owner, warning)| owner == "guest"
                    && warning.key() == "a".repeat(4095)
                    && warning.args() == vec!["a".repeat(4095); 20])
        );
    }

    #[test]
    fn refusal_rolls_back_only_its_owner_and_preserves_the_reason() {
        let mods = [
            ModEntry::new("healthy", Box::new(Healthy)),
            ModEntry::new("broken", Box::new(Refuses)),
            ModEntry::new("later", Box::new(Healthy)),
        ];
        let registrar = super::super::register_all(&mods, |_| false);
        assert_eq!(
            registrar.refused,
            ["mod broken refused registration: bootstrap failed: café\nmissing bridge"]
        );
        assert_eq!(
            registrar
                .biome_regions
                .iter()
                .map(|d| (d.mod_id.as_str(), d.id.as_str(), d.region.weight))
                .collect::<Vec<_>>(),
            [("healthy", "healthy:kept", 5), ("later", "later:kept", 5)]
        );
        assert!(registrar.block_entity_types.is_empty());
        assert!(registrar.block_entity_transforms.is_empty());
        assert!(registrar.parts.block_entity_kinds.is_empty());
        let keys = registrar.registry_keys.borrow();
        assert!(keys.declared_fluids.contains_key("healthy:soup"));
        assert!(keys.declared_fluids.contains_key("later:soup"));
        assert!(!keys.declared_fluids.contains_key("broken:before"));
        assert!(!keys.declared_fluids.contains_key("foreign:after"));
    }
}
