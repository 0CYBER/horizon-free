//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

impl PartRegistrar {
    /// New identifiers only, not attachment targets: Parts may attach to
    /// vanilla or another mod's existing content.
    pub(crate) fn owned_declaration_id(&self, name: &str) -> Result<String, String> {
        owned_declaration_id(self.mod_id, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    fn with_porter(f: impl FnOnce()) {
        crate::load_state::with_test_package_inventory(
            vec![PackageInfo::new(
                "porter".into(),
                "1".into(),
                PackageKind::Code,
                Side::Both,
                vec!["porter".into(), "legacy".into()],
                vec![],
            )],
            f,
        );
    }

    /// A Press whose state has no codec: kept for the session by a native mod.
    struct Session;
    #[mod_api::horizon_part]
    impl BlockPart for Session {}
    #[mod_api::horizon_part]
    impl BlockEntityPart for Session {}
    static SESSION: AttachmentType<u32> = AttachmentType {
        namespace: "porter",
        name: "session",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };
    impl BlockEntityPartState for Session {
        type Data = u32;
        const DATA: &'static AttachmentType<u32> = &SESSION;
    }
    struct DeclaresSession;
    #[mod_api::__subscriptions]
    impl Hooks for DeclaresSession {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let kind = r.block_entity_type("press").blocks(&[]).build();
            let _ = r.block_entity_part_with_state(&kind, Session);
        }
    }

    /// The codec rule follows how the game loaded the mod, not what the mod says of itself:
    /// the same registration is accepted from a native entry and refused from a wasm one.
    #[test]
    fn a_wasm_mods_codec_less_state_is_refused_by_how_the_game_loaded_it() {
        with_porter(|| {
            let native = [ModEntry::new("porter", Box::new(DeclaresSession))];
            assert!(
                super::super::register_all(&native, |_| false)
                    .refused
                    .is_empty()
            );
            let wasm = [ModEntry::wasm("porter", Box::new(DeclaresSession))];
            let refused = super::super::register_all(&wasm, |_| false).refused;
            assert_eq!(refused.len(), 1, "{refused:?}");
            assert!(
                refused[0].contains("porter:session") && refused[0].contains("needs a codec"),
                "{}",
                refused[0]
            );
        });
    }

    #[test]
    fn part_state_keys_stay_in_owned_namespaces_with_one_type_each() {
        const fn key<T>(namespace: &'static str, default: fn() -> T) -> AttachmentType<T> {
            AttachmentType {
                namespace,
                name: "progress",
                default,
                codec: None,
                copy_on_death: false,
                sync: false,
            }
        }
        with_porter(|| {
            let mut r = PartRegistrar::new("porter");
            let declare = |r: &mut PartRegistrar, declaration| {
                r.declare_block_entity_part_state("porter::Press", declaration)
            };
            let progress = PartStateDeclaration::of(&key("legacy", || 0_u32));
            assert_eq!(declare(&mut r, progress), Ok(()));
            assert_eq!(
                declare(&mut r, progress),
                Ok(()),
                "one key, one type, two parts"
            );
            let wider = PartStateDeclaration::of(&key("legacy", || 0_i64));
            assert_eq!(
                declare(&mut r, wider),
                Err(wider.refused(
                    "porter::Press",
                    PartStateRefusal::Incompatible { declared: "u32" }
                ))
            );
            assert_eq!(
                declare(
                    &mut r,
                    PartStateDeclaration::of(&key("minecraft", || 0_u32))
                ),
                Err(PartStateDeclaration::of(&key("minecraft", || 0_u32))
                    .refused("porter::Press", PartStateRefusal::UnownedNamespace))
            );
            assert_eq!(
                r.refused.len(),
                2,
                "each refusal is the mod's registration refusal"
            );
            assert!(r.refused[1].contains("minecraft:progress"));
        });
    }

    #[test]
    fn manifest_owned_declarations_keep_the_persisted_namespace() {
        with_porter(|| {
            let mut r = PartRegistrar::new("porter");
            r.dimension_shape = |_| true;
            assert_eq!(r.owned_declaration_id("bare").unwrap(), "porter:bare");
            assert_eq!(r.owned_declaration_id("legacy:pot").unwrap(), "legacy:pot");
            let kind = r.declare_block_entity_type("legacy:pot", vec!["legacy:pot".into()]);
            assert_ne!(
                kind,
                mod_vocab::__private::parts_installation::block_entity_type_key(u32::MAX)
            );
            assert_eq!(r.block_entity_types[0].id, "legacy:pot");
            let fluid = r.declare_fluid("legacy:soup");
            assert_ne!(
                fluid,
                mod_vocab::__private::parts_installation::fluid_key(u32::MAX)
            );
            assert!(
                r.registry_keys
                    .borrow()
                    .declared_fluids
                    .contains_key("legacy:soup")
            );
            let world_key = r.declare_world_fluid("legacy:broth", {
                let mut value: WorldFluid = Default::default();
                value.tick_delay = 20;
                value.drop_off = 2;
                value.slope_find_distance = 3;
                value.source_conversion = true;
                value.bucket = false;
                value
            });
            assert_ne!(
                world_key,
                mod_vocab::__private::parts_installation::fluid_key(u32::MAX)
            );
            assert_eq!(world_key, r.fluid_key("legacy:broth"));
            let keys = r.registry_keys.borrow();
            assert!(keys.declared_fluids.contains_key("legacy:broth"));
            assert!(keys.declared_fluids.contains_key("legacy:flowing_broth"));
            assert!(!keys.declared_fluids.contains_key("porter:broth"));
            drop(keys);
            r.declare_attribute(
                "legacy:stamina",
                1.0,
                0.0,
                10.0,
                true,
                Sentiment::Neutral,
                Default::default(),
            )
            .unwrap();
            r.declare_entity_attributes("legacy:cook", "minecraft:zombie", vec![])
                .unwrap();
            r.add_dimension("legacy:kitchen", DimensionSettings::default());
            assert!(r.recipe_book_type("legacy:kitchen").is_ok());
            assert_eq!(r.attributes[0].id, "legacy:stamina");
            assert_eq!(r.entity_attributes[0].id, "legacy:cook");
            assert_eq!(&*r.dimensions[0].id, "legacy:kitchen");
            assert!(r.refused.is_empty(), "{:?}", r.refused);
            assert_eq!(
                r.declare_block_entity_type("unowned:pot", vec![]),
                mod_vocab::__private::parts_installation::block_entity_type_key(u32::MAX)
            );
            let reason = r.refused.last().unwrap();
            assert!(
                reason.contains("porter") && reason.contains("unowned"),
                "{reason}"
            );
            assert!(r.owned_declaration_id("legacy:Bad Name").is_err());
        });
    }

    #[test]
    fn bare_block_entity_type_blocks_are_the_declaring_mods() {
        with_porter(|| {
            let mut r = PartRegistrar::new("porter");
            (&mut r as &mut dyn Registrar)
                .block_entity_type("press")
                .blocks(&["press", "legacy:pot", "minecraft:stone"])
                .build();
            assert!(r.refused.is_empty(), "{:?}", r.refused);
            assert_eq!(r.block_entity_types[0].id, "porter:press");
            assert_eq!(
                r.block_entity_types[0].blocks,
                ["porter:press", "legacy:pot", "minecraft:stone"]
            );
        });
    }

    #[test]
    fn blocks_and_items_are_attributed_before_flattening_mod_outputs() {
        struct Declares;
        #[crate::__subscriptions]
        impl Hooks for Declares {
            fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
                event.blocks.push({
                    let mut value: Block = Block::default();
                    value.id = "legacy:pot".into();
                    value.template = "minecraft:stone".into();
                    value.block_item = Some("spoon".into());
                    value.pick_item = Some("minecraft:stick".into());
                    value.wall_block = Some("legacy:wall_pot".into());
                    value.base_block = Some("soil".into());
                    value
                });
                event.blocks.push({
                    let mut value: Block = Block::default();
                    value.id = "unowned:pot".into();
                    value.template = "minecraft:stone".into();
                    value
                });
            }
            fn items(&self, _: &RegistrationContext, event: &mut ItemsEvent<'_>) {
                event.items.push({
                    let mut value: Item = Item::default();
                    value.id = "spoon".into();
                    value.template = "minecraft:stick".into();
                    value.crafting_remainder = Some("bowl".into());
                    value.block = Some("minecraft:stone".into());
                    value
                });
            }
        }
        let mods: Mods = vec![ModEntry::new("porter", Box::new(Declares))].into();
        crate::testing::with_mods(mods, || {
            with_porter(|| {
                let context = RegistrationContext::new();
                let mut blocks = Vec::new();
                let mut items = Vec::new();
                crate::dispatch_blocks(&context, &mut blocks);
                crate::dispatch_items(&context, &mut items);
                assert_eq!(blocks[0].id, "legacy:pot");
                assert_eq!(items[0].id, "porter:spoon");
                // A bare reference is the declaring mod's, as a bare id is; a full one is kept.
                let pot = &blocks[0];
                assert_eq!(pot.block_item.as_deref(), Some("porter:spoon"));
                assert_eq!(pot.pick_item.as_deref(), Some("minecraft:stick"));
                assert_eq!(pot.wall_block.as_deref(), Some("legacy:wall_pot"));
                assert_eq!(pot.base_block.as_deref(), Some("porter:soil"));
                assert_eq!(items[0].crafting_remainder.as_deref(), Some("porter:bowl"));
                assert_eq!(items[0].block.as_deref(), Some("minecraft:stone"));
                // Invalid output stays visible for an outer host dispatch to refuse
                // too, rather than being silently lost inside a wasm guest.
                assert_eq!(blocks[1].id, "unowned:pot");
                let errors = take_declaration_refusals(&context);
                assert_eq!(errors.len(), 1);
                assert!(errors[0].contains("porter") && errors[0].contains("unowned"));
            })
        });
    }
}
