use super::*;
/// An entity type's initial attribute set, declared in code (NeoForge's
/// `EntityAttributeCreationEvent`): the type's id (`mymod:ghoul`), the
/// vanilla type its content file copies (`minecraft:zombie`, whose row the
/// set starts from, as `Zombie.createAttributes` builds on the template's)
/// and each attribute's base value that differs from the template's. The
/// game resolves the set onto the type's attribute row once, at load.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: only registration builds it; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct EntityAttributeDeclaration {
    /// The entity type the set belongs to.
    pub id: String,
    /// The template the type copies; its row is the set's start.
    pub template: String,
    /// Each attribute's id (`minecraft:max_health`) and its base.
    pub attributes: Vec<(String, f64)>,
}

/// The entity attribute sets mods declared.
pub fn entity_attribute_declarations() -> &'static [EntityAttributeDeclaration] {
    installed().map_or(&[], |installed| &installed.entity_attributes)
}

/// Attributes a mod adds to an existing entity type's default set (NeoForge's
/// `EntityAttributeModificationEvent`): the type (`minecraft:zombie`) and each
/// attribute's id with its base, or none for the attribute's own default (the
/// event's one-argument `add`). The game puts them on the type's attribute row
/// once, at load, after every declared set, so new entities of the type have
/// them; live entities are not edited.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: only registration builds it; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct EntityAttributeModification {
    /// The entity type whose default set grows.
    pub entity: String,
    /// Each attribute's id (`minecraft:luck`) and its base, if not the default.
    pub attributes: Vec<(String, Option<f64>)>,
}

/// The attribute additions mods made to existing entity types, in
/// registration order.
pub fn entity_attribute_modifications() -> &'static [EntityAttributeModification] {
    installed().map_or(&[], |installed| &installed.entity_attribute_modifications)
}

/// Registrar operations for entity attributes.
pub trait EntityAttributesRegistrar {
    /// Declare the initial attribute set of the mod's entity type `path`
    /// (NeoForge's `EntityAttributeCreationEvent`), starting from `template`'s
    /// (`minecraft:zombie`, the one the type's content file copies, as
    /// `Zombie.createAttributes` builds on the template's): each attribute's
    /// id (`minecraft:max_health`) and the base value that differs from the
    /// template's. The values reach the type's own fields, its live attribute
    /// map and the attribute sync, so attribute reads answer the
    /// same defaults with no second copy.
    ///
    /// Resolved once, at load: an id outside the mod's manifest-owned namespaces, a duplicate
    /// attribute, a non-finite base or a second declaration of the same type
    /// refuses the mods, naming the mod; an attribute the game does not know,
    /// or a template without attribute defaults, refuses the resource load,
    /// naming the type.
    fn declare_entity_attributes(
        &mut self,
        path: &str,
        template: &str,
        attributes: Vec<(String, f64)>,
    ) -> Result<(), crate::EntityAttributeError>;

    /// Add attributes to an existing entity type's default set (NeoForge's
    /// `EntityAttributeModificationEvent`): `entity` is any type with an
    /// attribute row (`minecraft:zombie`; a bare name is in `minecraft`): a
    /// vanilla type, or a mod's type with a declared set. A living type that
    /// only a content file defines has no row of its own yet; declare its set
    /// with [`declare_entity_attributes`](Self::declare_entity_attributes)
    /// first, and the additions apply on top of it. Each attribute's id
    /// (`minecraft:luck`) takes its base, or with none the attribute's own
    /// default, as the event's one-argument `add`. An attribute the type
    /// already has takes the new base. The change is the type's default:
    /// entities created after load have it, through their spawn fields, their
    /// live attribute map and the attribute sync; nothing edits a live entity.
    ///
    /// Applied once, at load, after every declared set
    /// ([`declare_entity_attributes`](Self::declare_entity_attributes)), in
    /// registration order, so a later addition of the same attribute wins.
    /// An invalid type id, the same attribute twice in one call or a
    /// non-finite base refuses the mods, naming the mod; an attribute the
    /// game does not know, or a type without an attribute row, refuses the
    /// resource load, naming the type.
    fn modify_entity_attributes(
        &mut self,
        entity: &str,
        attributes: Vec<(String, Option<f64>)>,
    ) -> Result<(), crate::EntityAttributeError>;
}
