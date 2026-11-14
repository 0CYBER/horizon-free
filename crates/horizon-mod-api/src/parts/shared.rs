//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

pub(crate) fn insert_part<P: ?Sized>(
    table: &mut HashMap<Box<str>, Vec<Attached<P>>>,
    id: &str,
    part: Attached<P>,
    refused: &mut Vec<String>,
) -> bool {
    let list = table.entry(id.into()).or_default();
    if let Some(other) = list.iter().find(|other| other.type_name == part.type_name) {
        refused.push(format!(
            "{} is attached to {id} twice, by {} and {}",
            part.type_name, other.mod_id, part.mod_id
        ));
        return false;
    }
    list.push(part);
    true
}

thread_local! {
    // A reference to the registrar's canonical requests while Hooks::register is in flight.
    // The table is moved, not copied, into Installed when registration finishes.
    pub(crate) static REGISTERING_KEYS: RefCell<Option<Rc<RefCell<horizon_mod_runtime::registry_keys::Requests>>>> = const { RefCell::new(None) };
}

pub(crate) fn with_registering_keys<T>(
    read: impl FnOnce(Option<&horizon_mod_runtime::registry_keys::Requests>) -> T,
) -> T {
    REGISTERING_KEYS.with(|keys| match keys.borrow().as_ref() {
        Some(keys) => read(Some(&keys.borrow())),
        None => read(None),
    })
}

// no wire: process-local registration and dispatch storage contains trait objects and owning-mod identities.
/// The game's [`Registrar`]: every mod's registrations, checked as they come.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct PartRegistrar {
    pub(crate) biome_modifiers: Vec<BiomeModifierDeclaration>,
    pub(crate) biome_regions: Vec<BiomeRegionDeclaration>,
    pub(crate) datapack_registries: Vec<crate::DatapackRegistryDeclaration>,
    pub(crate) menu_recipe_books: HashMap<String, MenuBookDeclaration>,
    pub(crate) self_refusal: Option<String>,
    /// Loading warnings, by owning mod id, in registration order.
    pub(crate) warnings: Vec<(String, crate::LoadingMessage)>,
    pub(crate) recipe_book_types: HashMap<String, String>,
    pub(crate) game_rule_categories: Vec<String>,
    pub(crate) game_rules: Vec<crate::GameRuleDecl>,
    pub(crate) custom_stats: Vec<String>,
    pub(crate) mod_id: &'static str,
    pub(crate) mod_index: usize,
    /// Whether the registering mod was loaded as a wasm module.
    pub(crate) mod_wasm: bool,
    /// Parts registered so far, by every mod: the next part's `order`.
    pub(crate) registered: usize,
    pub(crate) parts: Parts,
    /// Aggregate selector metadata bytes during this registration pass.
    pub(crate) entity_selector_bytes: usize,
    pub(crate) tags: TagParts,
    pub(crate) block_entity_types: Vec<BlockEntityTypeDecl>,
    pub(crate) block_entity_transforms: HashMap<String, Arc<dyn crate::BlockEntityTransform>>,
    pub(crate) entity_attributes: Vec<EntityAttributeDeclaration>,
    pub(crate) world_fluids: Vec<(String, crate::WorldFluid)>,
    pub(crate) entity_attribute_modifications: Vec<EntityAttributeModification>,
    pub(crate) attributes: Vec<AttributeDeclaration>,
    pub(crate) attribute_widenings: Vec<AttributeBoundsWidening>,
    pub(crate) general_entity_types: Vec<GeneralEntityTypeDeclaration>,
    pub(crate) spawn_placements: Vec<SpawnPlacementDeclaration>,
    pub(crate) registry_keys: Rc<RefCell<horizon_mod_runtime::registry_keys::Requests>>,
    pub(crate) effects: OnceLock<horizon_mod_runtime::registry_keys::Effects>,
    pub(crate) controllers: Vec<ControllerInfo>,
    pub(crate) arm_poses: Vec<PoseInfo>,
    pub(crate) structure_types: Vec<StructureTypeInfo>,
    pub(crate) dimensions: Vec<DimensionInfo>,
    pub(crate) dimension_shape: fn(&crate::DimensionSettings) -> bool,
    pub(crate) chunk_generators: Vec<ChunkGeneratorInfo>,
    pub(crate) biome_sources: Vec<BiomeSourceInfo>,
    pub(crate) refused: Vec<String>,
    /// The Part state keys this mod declared, with their types' Wire layouts and names.
    pub(crate) part_states: HashMap<(&'static str, &'static str), (u64, &'static str)>,
}

impl PartRegistrar {
    pub(crate) fn new(mod_id: &'static str) -> Self {
        let _ = crate::__private::initialize();
        Self {
            menu_recipe_books: HashMap::new(),
            self_refusal: None,
            warnings: Vec::new(),
            biome_modifiers: Vec::new(),
            biome_regions: Vec::new(),
            datapack_registries: Vec::new(),
            recipe_book_types: HashMap::new(),
            game_rule_categories: Vec::new(),
            game_rules: Vec::new(),
            custom_stats: Vec::new(),
            mod_id,
            mod_index: 0,
            mod_wasm: false,
            registered: 0,
            parts: Self::capability_parts(),
            entity_selector_bytes: 0,
            tags: TagParts::default(),
            block_entity_types: Vec::new(),
            block_entity_transforms: HashMap::new(),
            entity_attributes: Vec::new(),
            world_fluids: Vec::new(),
            entity_attribute_modifications: Vec::new(),
            attributes: Vec::new(),
            attribute_widenings: Vec::new(),
            general_entity_types: Vec::new(),
            spawn_placements: Vec::new(),
            registry_keys: Default::default(),
            effects: OnceLock::new(),
            controllers: Vec::new(),
            arm_poses: Vec::new(),
            structure_types: Vec::new(),
            dimensions: Vec::new(),
            dimension_shape: |_| false,
            chunk_generators: Vec::new(),
            biome_sources: Vec::new(),
            refused: Vec::new(),
            part_states: HashMap::new(),
        }
    }
    /// `part`, of kind `kind`, for tag `tag`; none if it is refused.
    pub(crate) fn tagged<P: ?Sized>(
        &mut self,
        kind: &'static str,
        tag: &str,
        part: Arc<P>,
    ) -> Option<(Box<str>, Attached<P>)> {
        if tag.starts_with('#') || !namespaced(tag) {
            self.refused.push(format!(
                "{} attached {kind} to tag {tag:?}, which is not a namespaced tag name (write it without #)",
                self.mod_id
            ));
            return None;
        }
        self.registered += 1;
        Some((
            tag.into(),
            Attached {
                mod_id: self.mod_id,
                mod_index: self.mod_index,
                order: self.registered,
                type_name: kind,
                part,
            },
        ))
    }

    /// Attach `part`, of kind `kind`, to `id` in `table`; false if it is refused.
    pub(crate) fn attach<P: ?Sized>(
        &mut self,
        table: fn(&mut Parts) -> &mut HashMap<Box<str>, Vec<Attached<P>>>,
        kind: &'static str,
        id: &str,
        part: Arc<P>,
    ) -> bool {
        if !namespaced(id) {
            self.refused.push(format!(
                "{} attached {kind} to {id:?}, which is not a namespaced id",
                self.mod_id
            ));
            return false;
        }
        self.attach_known(table, kind, id, part)
    }

    pub(crate) fn attach_known<P: ?Sized>(
        &mut self,
        table: fn(&mut Parts) -> &mut HashMap<Box<str>, Vec<Attached<P>>>,
        kind: &'static str,
        id: &str,
        part: Arc<P>,
    ) -> bool {
        let part = Attached {
            mod_id: self.mod_id,
            mod_index: self.mod_index,
            order: self.registered + 1,
            type_name: kind,
            part,
        };
        let inserted = insert_part(table(&mut self.parts), id, part, &mut self.refused);
        if inserted {
            self.registered += 1;
        }
        inserted
    }
}

pub(crate) fn namespaced(id: &str) -> bool {
    id.split_once(':')
        .is_some_and(|(namespace, path)| !namespace.is_empty() && !path.is_empty())
}

/// Every mod's [`Hooks::register`](crate::Hooks::register) run against one registrar, in dispatch order.
pub(crate) fn register_all(
    mods: &[ModEntry],
    dimension_shape: fn(&crate::DimensionSettings) -> bool,
) -> PartRegistrar {
    let mut registrar = PartRegistrar::new(mods.first().map_or("horizon", |entry| {
        mod_vocab::__private::parts_installation::mod_entry_id(entry)
    }));
    registrar.dimension_shape = dimension_shape;
    struct RestoreKeys(Option<Rc<RefCell<horizon_mod_runtime::registry_keys::Requests>>>);
    impl Drop for RestoreKeys {
        fn drop(&mut self) {
            REGISTERING_KEYS.set(self.0.take());
        }
    }
    let _active = ActiveRegistration::enter();
    let _scope = RestoreKeys(REGISTERING_KEYS.replace(Some(registrar.registry_keys.clone())));
    for (index, hooks) in mod_vocab::__private::entered_registration(mods, "register").enumerate() {
        registrar.mod_id = mod_vocab::__private::parts_installation::mod_entry_id(&mods[index]);
        registrar.mod_index = index;
        registrar.mod_wasm = mod_vocab::__private::parts_installation::mod_entry_wasm(&mods[index]);
        let context = crate::RegistrationContext::new();
        let rollback = registrar.registration_checkpoint();
        hooks.register(&mut registrar, &context);
        for warning in mod_vocab::__private::take_registration_warnings(&context) {
            RegistrationRefusalRegistrar::registration_warning(&mut registrar, warning);
        }
        let reason = mod_vocab::__private::take_registration_refusal(&context)
            .or_else(|| registrar.self_refusal.take());
        if let Some(reason) = reason {
            rollback(&mut registrar);
            registrar.refused.push(format!(
                "mod {} refused registration: {reason}",
                mod_vocab::__private::parts_installation::mod_entry_id(&mods[index])
            ));
        }
    }
    registrar.expand_block_entity_parts();
    registrar.refuse_unknown_generators_and_biome_sources();
    registrar
}

/// Builds the parts from every mod's [`Hooks::register`](crate::Hooks::register), in dispatch order. Once, at startup;
/// the error lists every part refused, naming its mods.
pub fn install_parts(
    mods: &[ModEntry],
    schema: &mod_vocab::__private::RuleSchema<'_>,
    dimension_shape: fn(&crate::DimensionSettings) -> bool,
) -> Result<(), Vec<String>> {
    install_registrar(register_all(mods, dimension_shape), schema)
}

pub(crate) fn install_registrar(
    mut registrar: PartRegistrar,
    schema: &mod_vocab::__private::RuleSchema<'_>,
) -> Result<(), Vec<String>> {
    let slots = schema.validate(&registrar.game_rule_categories, &registrar.game_rules);
    if let Err(errors) = &slots {
        registrar.refused.extend(errors.iter().cloned());
    }
    for (id, warning) in &registrar.warnings {
        log::warn!(
            "mod {id} loading warning: {} {:?}",
            warning.key(),
            warning.args()
        );
    }
    registrar.validate_recipe_types();
    if !registrar.refused.is_empty() {
        return Err(registrar.refused);
    }
    let game_rule_slots = slots.expect("every schema refusal was collected");
    let installed = Installed {
        biome_modifiers: registrar.biome_modifiers,
        biome_regions: {
            let mut regions = registrar.biome_regions;
            regions.sort_by(|a, b| (&a.mod_id, &a.id).cmp(&(&b.mod_id, &b.id)));
            regions
        },
        datapack_registries: registrar.datapack_registries,
        menu_recipe_books: registrar.menu_recipe_books,
        recipe_book_types: registrar.recipe_book_types,
        game_rule_slots,
        game_rules: registrar.game_rules,
        game_rule_categories: registrar.game_rule_categories,
        custom_stats: registrar.custom_stats,
        keys: registrar.parts,
        tags: registrar.tags,
        block_entity_types: registrar.block_entity_types,
        block_entity_transforms: registrar.block_entity_transforms,
        entity_attributes: registrar.entity_attributes,
        world_fluids: registrar.world_fluids,
        attributes: registrar.attributes,
        attribute_widenings: registrar.attribute_widenings,
        general_entity_types: registrar.general_entity_types,
        entity_attribute_modifications: registrar.entity_attribute_modifications,
        spawn_placements: registrar.spawn_placements,
        resolved_block_entity_types: OnceLock::new(),
        registry_keys: Rc::try_unwrap(registrar.registry_keys)
            .unwrap_or_else(|_| unreachable!("registration scope has ended"))
            .into_inner(),
        effects: registrar.effects,
        fluids: OnceLock::new(),
        resolved_items: OnceLock::new(),
        resolved_entity_types: OnceLock::new(),
        resolved_blocks: OnceLock::new(),
        particles: OnceLock::new(),
        controllers: registrar.controllers,
        arm_poses: registrar.arm_poses,
        structure_types: registrar.structure_types,
        dimensions: registrar.dimensions,
        chunk_generators: registrar.chunk_generators,
        biome_sources: registrar.biome_sources,
        warnings: registrar.warnings,
    };
    mod_vocab::__private::parts_installation::install(installed);

    Ok(())
}

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    use super::super::items::tests::Cell;
    use super::mark_game_thread;
    use super::*;
    use crate::parts::tests::*;
    use crate::*;

    use std::sync::Arc;

    include!(concat!(env!("OUT_DIR"), "/shared_fixtures.rs"));
}
