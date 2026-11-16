//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

impl PartRegistrar {
    pub(crate) fn expand_block_entity_parts(&mut self) {
        for (id, parts) in &self.parts.block_entity_kinds {
            let Some(kind) = self.block_entity_types.iter().find(|kind| kind.id == **id) else {
                for part in parts {
                    self.refused.push(format!(
                        "mod {} attached {} to undeclared native block entity type {id}",
                        part.mod_id, part.type_name
                    ));
                }
                continue;
            };
            for block in &kind.blocks {
                for part in parts {
                    let block_part: Arc<dyn BlockPart> = part.part.clone();
                    if insert_part(
                        &mut self.parts.blocks,
                        block,
                        part.with_part(block_part),
                        &mut self.refused,
                    ) {
                        insert_part(
                            &mut self.parts.block_entities,
                            block,
                            part.clone(),
                            &mut self.refused,
                        );
                    }
                }
            }
        }
        for parts in self.parts.blocks.values_mut() {
            parts.sort_by_key(|part| (part.mod_index, part.order));
        }
        for parts in self.parts.block_entities.values_mut() {
            parts.sort_by_key(|part| (part.mod_index, part.order));
        }
    }
}

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use super::*;
    #[cfg(test)]
    use crate::parts::tests::*;
    use crate::*;

    #[test]
    pub(crate) fn block_entity_slots_resolve_forward_declarations_and_refuse_unknown() {
        struct Request(Arc<std::sync::Mutex<Vec<BlockEntityTypeKey>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Request {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let later = r.block_entity_type_key("owner:press");
                assert_eq!(later, r.block_entity_type_key("owner:press"));
                assert_eq!(
                    mod_vocab::__private::parts_installation::validate_block_entity_type_key(
                        mod_vocab::__private::parts_installation::block_entity_type_key_slot(later)
                    ),
                    Ok(later)
                );
                assert!(
                    mod_vocab::__private::parts_installation::validate_block_entity_type_key(
                        mod_vocab::__private::parts_installation::block_entity_type_key_slot(later)
                            + 1
                    )
                    .is_err()
                );
                self.0.lock().unwrap().push(later);
            }
        }
        struct Declare(Arc<std::sync::Mutex<Vec<BlockEntityTypeKey>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Declare {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let declared = r.block_entity_type("press").build();
                let chest = r.block_entity_type_key("chest");
                self.0.lock().unwrap().extend([declared, chest]);
            }
        }
        let keys = Arc::new(std::sync::Mutex::new(Vec::new()));
        install_parts(
            &[
                ModEntry::new("first", Box::new(Request(keys.clone()))),
                ModEntry::new("owner", Box::new(Declare(keys.clone()))),
            ],
            &crate::parts::tests::rule_schema(),
            |_| false,
        )
        .unwrap();
        let types = |names: &[&str]| names.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            {
                let types = &types(&["minecraft:chest"]);
                mod_vocab::__private::resolve_block_entity_types(
                    types.iter().map(String::as_str),
                    |id| types.iter().position(|name| name == id),
                )
            }
            .unwrap_err(),
            [
                "mod first requested unknown block entity type owner:press",
                "mod owner requested unknown block entity type owner:press"
            ]
        );
        {
            let types = &types(&["minecraft:chest", "owner:press", "minecraft:furnace"]);
            mod_vocab::__private::resolve_block_entity_types(
                types.iter().map(String::as_str),
                |id| types.iter().position(|name| name == id),
            )
        }
        .unwrap();
        let keys = keys.lock().unwrap();
        assert_eq!(keys[0], keys[1]);
        assert_ne!(keys[0], keys[2]);
        assert_eq!(
            mod_vocab::__private::block_entity_type_key("owner:press"),
            Some(keys[0])
        );
        assert_eq!(
            mod_vocab::__private::block_entity_type_name(keys[2]),
            Some("minecraft:chest")
        );
        let furnace = mod_vocab::__private::block_entity_type_key("minecraft:furnace").unwrap();
        assert_ne!(furnace, keys[2]);
        assert_eq!(
            mod_vocab::__private::parts_installation::validate_block_entity_type_key(
                mod_vocab::__private::parts_installation::block_entity_type_key_slot(furnace)
            ),
            Ok(furnace)
        );
        assert!(
            mod_vocab::__private::parts_installation::validate_block_entity_type_key(u32::MAX)
                .is_err()
        );
        struct Table;
        impl mod_api::prelude::KeysBackend for Table {
            fn block_entity_type_key_id(&self, key: BlockEntityTypeKey) -> String {
                mod_vocab::__private::block_entity_type_name(key)
                    .unwrap()
                    .to_owned()
            }
        }
        impl Backend for Table {}
        mod_api::impl_backend_areas!(Table, except(keys));
        impl Host for Table {
            fn backend(&self) -> Option<BackendRef<'_>> {
                Some(mod_vocab::__private::parts_installation::backend_ref(self))
            }
        }
        mod_api::impl_host_areas!(Table);
        assert_eq!(keys[0].id(&Table), "owner:press");
    }

    #[test]
    pub(crate) fn invalid_block_entity_type_paths_refuse_registration() {
        struct Invalid;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Invalid {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                for path in ["", "other:press", "Uppercase", "has space"] {
                    r.block_entity_type(path).build();
                }
            }
        }
        let errors = install_parts(
            &[ModEntry::new("test", Box::new(Invalid))],
            &crate::parts::tests::rule_schema(),
            |_| false,
        )
        .unwrap_err();
        assert_eq!(errors.len(), 4);
        assert!(errors.iter().all(|error| error.contains("mod test")
            && (error.contains("invalid identifier") || error.contains("unowned namespace"))));
    }

    /// The block entity of type `doubler:press_entity`, held by block `test:press`: it ticks, and
    /// each tick adds a data slot.
    pub(crate) struct Press;
    #[mod_api::horizon_part]
    impl BlockPart for Press {
        fn analog_output(&self, _: &dyn LevelHost, event: &mut AnalogOutputEvent) {
            event.output += 40;
        }
    }
    #[mod_api::horizon_part]
    impl BlockEntityPart for Press {
        fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
            event.ticks = true;
        }
        fn block_entity_tick(&self, _: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
            event.entity.data.push(1);
        }
        fn block_entity_view_distance(
            &self,
            _: &dyn ClientHost,
            event: &mut BlockEntityViewDistanceEvent<'_>,
        ) {
            event.distance = 128.0;
        }
    }
    /// The gold a `doubler:hoard` block entity holds, kept beside it.
    pub const HOARD: AttachmentType<u32> = AttachmentType {
        namespace: "test",
        name: "hoard",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };
    /// How many times a `doubler:hoard` was asked about its removal.
    pub static HOARD_ASKED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    /// The block entity of type `doubler:hoard`, held by block `test:hoard`: when it is
    /// removed it gives back the gold its `HOARD` counts.
    pub(crate) struct Hoard(pub(crate) BlockKey);
    #[mod_api::horizon_part]
    impl BlockPart for Hoard {}
    #[mod_api::horizon_part]
    impl BlockEntityPart for Hoard {
        /// With data `[1]`, it replaces its own block with granite.
        fn block_entity_tick(
            &self,
            host: &mut dyn ServerHost,
            event: &mut BlockEntityTickEvent<'_>,
        ) {
            if event.entity.data == [1] {
                let level = host.level("minecraft:overworld").unwrap();
                let state = BlockState::of(&host.block_states(), self.0, &[]).unwrap();
                assert_eq!(
                    level.set_block(host, event.pos, state, BlockUpdateFlags::KNOWN_SHAPE),
                    Ok(true)
                );
            }
        }
        fn pre_remove_side_effects(
            &self,
            level: &mut dyn LevelRandomHost,
            event: &mut PreRemoveSideEffectsEvent<'_>,
        ) {
            HOARD_ASKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let BlockPos { x, y, z } = event.pos;
            if let Ok(Some(count @ 1..)) = HOARD.get_existing(level, Holder::BlockEntity([x, y, z]))
            {
                event.drops.push(crate::parts::items::tests::stack(
                    "minecraft:gold_ingot",
                    count,
                    0,
                ));
            }
        }
    }

    #[test]
    pub(crate) fn a_vanilla_type_refuses_a_native_payload_part_at_registration() {
        let mut r = super::PartRegistrar::new("test");
        let kind = r.block_entity_type_key("minecraft:chest");
        assert_eq!(
            (&mut r as &mut dyn Registrar).block_entity_part(&kind, Press),
            Err(BlockEntityPartError::VanillaType { kind }),
        );
        assert!(r.parts.block_entity_kinds.is_empty());
        assert!(r.parts.blocks.is_empty());
        assert!(r.parts.block_entities.is_empty());
    }

    #[test]
    pub(crate) fn a_duplicate_native_part_returns_its_typed_error_and_keeps_the_first() {
        let mut r = super::PartRegistrar::new("test");
        let kind = (&mut r as &mut dyn Registrar)
            .block_entity_type("press")
            .blocks(&["test:press"])
            .build();
        (&mut r as &mut dyn Registrar)
            .block_entity_part(&kind, Press)
            .unwrap();
        assert_eq!(
            (&mut r as &mut dyn Registrar).block_entity_part(&kind, Press),
            Err(BlockEntityPartError::Duplicate {
                kind,
                part: type_name::<Press>()
            }),
        );
        assert_eq!(r.parts.block_entity_kinds["test:press"].len(), 1);
    }

    #[test]
    pub(crate) fn a_block_entity_part_answers_by_block_and_by_entity_type() {
        game_thread();
        let ticks = |block| {
            let mut ticks = false;
            dispatch_block_entity_ticks(&NoHost, block, &mut ticks);
            ticks
        };
        // The ticker is asked by block: the type id is not a block.
        assert!(ticks("test:press"));
        assert!(!ticks("doubler:press_entity"));
        let tick = |kind: &str| {
            let mut entity = ModBlockEntity::new(kind.into(), Default::default());
            dispatch_block_entity_tick(
                &mut NoHost,
                &mut BlockEntityTickReply::Changed,
                [0, 64, 0],
                &mut entity,
            );
            entity.data.len()
        };
        // The tick is asked by the entity's type, which differs from its block's id.
        assert_eq!(tick("doubler:press_entity"), 1);
        assert_eq!(tick("test:press"), 0);
        // The view distance is asked by block, as the ticker is.
        let distance = |block| {
            let mut distance = 64.0;
            dispatch_block_entity_view_distance(&NoHost, block, &mut distance);
            distance
        };
        assert_eq!(distance("test:press"), 128.0);
        assert_eq!(distance("doubler:press_entity"), 64.0);
        assert_eq!(distance("test:other"), 64.0);
    }
}

impl BlockEntitiesRegistrar for PartRegistrar {
    fn set_block_entity_transform(
        &mut self,
        key: crate::BlockEntityTypeKey,
        step: Arc<dyn crate::BlockEntityTransform>,
    ) {
        let id = self
            .registry_keys
            .borrow()
            .block_entity_types
            .id(mod_vocab::__private::parts_installation::block_entity_type_key_slot(key))
            .map(str::to_owned);
        if let Some(id) = id.filter(|id| {
            self.block_entity_types.iter().any(|decl| decl.id == *id)
                && self.owned_declaration_id(id).is_ok()
        }) {
            self.block_entity_transforms.insert(id, step);
        } else {
            self.refused
                .push("transform requires an owned block entity declaration".into());
        }
    }

    fn block_entity_type_key(&mut self, id: &str) -> crate::BlockEntityTypeKey {
        match self
            .registry_keys
            .borrow_mut()
            .block_entity_types
            .request(self.mod_id, id)
        {
            Ok(slot) => mod_vocab::__private::parts_installation::block_entity_type_key(slot),
            Err(error) => {
                self.refused.push(error);
                mod_vocab::__private::parts_installation::block_entity_type_key(u32::MAX)
            }
        }
    }

    fn declare_block_entity_type(
        &mut self,
        path: &str,
        blocks: Vec<String>,
    ) -> crate::BlockEntityTypeKey {
        let id = match self.owned_declaration_id(path) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return mod_vocab::__private::parts_installation::block_entity_type_key(u32::MAX);
            }
        };
        let key = self.block_entity_type_key(&id);
        let blocks = blocks
            .iter()
            .map(|block| mod_vocab::__private::qualified(self.mod_id, block))
            .collect();
        self.block_entity_types
            .push(mod_vocab::__private::parts_installation::block_entity_declaration(id, blocks));
        key
    }

    fn attach_block_entity_part(
        &mut self,
        key: &crate::BlockEntityTypeKey,
        kind: &'static str,
        part: Arc<dyn BlockEntityPart>,
    ) -> Result<(), crate::BlockEntityPartError> {
        let id = self
            .registry_keys
            .borrow()
            .block_entity_types
            .id(mod_vocab::__private::parts_installation::block_entity_type_key_slot(*key))
            .map(str::to_owned)
            .unwrap_or_default();
        if id.starts_with("minecraft:") {
            return Err(crate::BlockEntityPartError::VanillaType { kind: *key });
        }
        if self.attach_known(|p| &mut p.block_entity_kinds, kind, &id, part) {
            Ok(())
        } else {
            Err(crate::BlockEntityPartError::Duplicate {
                kind: *key,
                part: kind,
            })
        }
    }

    fn declare_block_entity_part_state(
        &mut self,
        part: &'static str,
        declaration: crate::PartStateDeclaration,
    ) -> Result<(), crate::BlockEntityStateError> {
        self.check_part_state(declaration).map_err(|reason| {
            let error = declaration.refused(part, reason);
            self.refused.push(format!("{}: {error}", self.mod_id));
            error
        })
    }

    fn provide_block_entity_capability(
        &mut self,
        name: &'static str,
        kind: &crate::BlockEntityTypeKey,
        before_game: bool,
        provider: BlockProviderOf,
    ) {
        self.capability(
            |p| &mut p.capabilities,
            Some(|p| &p.block_capabilities),
            name,
            ProviderTarget::BlockEntity(kind),
            before_game,
            {
                let handler =
                    mod_vocab::__private::parts_installation::block_provider_handler(&provider);
                (provider, handler)
            },
        );
    }
}

impl PartRegistrar {
    /// Checks a Part state declaration of either holder and records its key: see
    /// `PartStateDeclaration::check`, a key outside this mod's namespaces, or one declared
    /// with another Wire layout.
    pub(crate) fn check_part_state(
        &mut self,
        declaration: crate::PartStateDeclaration,
    ) -> Result<(), crate::PartStateRefusal> {
        let key = (declaration.namespace, declaration.name);
        declaration
            .check(self.mod_wasm)
            .and_then(|()| {
                self.owned_declaration_id(&format!("{}:{}", key.0, key.1))
                    .map(drop)
                    .map_err(|_| crate::PartStateRefusal::UnownedNamespace)
            })
            .and_then(|()| match self.part_states.get(&key) {
                Some(&(layout, declared)) if layout != declaration.layout => {
                    Err(crate::PartStateRefusal::Incompatible { declared })
                }
                _ => Ok(()),
            })?;
        self.part_states
            .insert(key, (declaration.layout, declaration.data_name));
        Ok(())
    }
}
