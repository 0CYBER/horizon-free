//! Registered entity defaults and lazy attribute instances. Cross-attribute bulk
//! mutation/serialization order is deliberately outside this per-attribute API.
use super::{Definition, Instance, Observer};
use crate::{DATA_VERSION, compression::invalid};
use serde_json::Value;
use std::{collections::BTreeMap, convert::Infallible, fmt, io, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttributeId(usize);
impl AttributeId {
    pub(super) fn index(self) -> usize {
        self.0
    }
    /// The attribute's id in the `minecraft:attribute` registry: the catalog
    /// export lists `BuiltInRegistries.ATTRIBUTE` in order.
    pub fn registry_id(self) -> i32 {
        self.0 as i32
    }
}
#[derive(Clone, Debug)]
pub struct RegisteredAttribute {
    pub name: String,
    pub definition: Definition,
    pub syncable: bool,
    pub tooltip: super::Tooltip,
}
/// A mod's declared attribute type, as the game hands it to
/// [`Catalog::with_attributes`]: the declaring mod's id (the sort key), the
/// namespaced attribute id, its definition, sync flag and tooltip style.
#[derive(Clone, Debug)]
pub struct AddedAttribute {
    /// The declaring mod's id (`mymod`).
    pub mod_id: String,
    /// The attribute's namespaced id (`mymod:mana`).
    pub id: String,
    /// The attribute's definition: default and bounds.
    pub definition: Definition,
    /// Whether the attribute joins the sync set.
    pub syncable: bool,
    /// How its tooltip lines read: sentiment, display and base modifier.
    pub tooltip: super::Tooltip,
}
/// A mod's ask to widen an existing attribute's bounds, as the game hands it
/// to [`Catalog::with_widened_bounds`] (arismoko/horizon#3728).
#[derive(Clone, Debug)]
pub struct WidenedBounds {
    /// The asking mod's id, which a refusal names.
    pub mod_id: String,
    /// The attribute's namespaced id (`minecraft:max_health`).
    pub id: String,
    /// The asked lower bound.
    pub min: f64,
    /// The asked upper bound.
    pub max: f64,
}
/// An ask `Catalog::with_widened_bounds` refused, naming the mod.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WidenBoundsError {
    /// The catalog has no attribute with that id.
    UnknownAttribute { mod_id: String, id: String },
    /// The attribute's default is not inside the asked `[min, max]`.
    DefaultOutside { mod_id: String, id: String },
}
impl fmt::Display for WidenBoundsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownAttribute { mod_id, id } => {
                write!(
                    f,
                    "mod {mod_id} widens the bounds of {id}, which no attribute has"
                )
            }
            Self::DefaultOutside { mod_id, id } => write!(
                f,
                "mod {mod_id} widens the bounds of {id} to a range its default is not inside"
            ),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Catalog {
    attributes: Vec<RegisteredAttribute>,
    ids: BTreeMap<String, AttributeId>,
    defaults: BTreeMap<String, Arc<[Option<f64>]>>,
}
/// A row `Catalog::with_entity_row` or `Catalog::with_entity_additions`
/// refused: an override names an attribute the catalog does not know, or the
/// template (or, for additions, the entity type itself) has no row of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EntityRowError {
    UnknownAttribute { entity: String, attribute: String },
    UnknownTemplate { entity: String, template: String },
    UnknownEntity { entity: String },
}
impl fmt::Display for EntityRowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownAttribute { entity, attribute } => {
                write!(f, "entity {entity} has no attribute {attribute}")
            }
            Self::UnknownTemplate { entity, template } => {
                write!(
                    f,
                    "entity {entity} copies {template}, which has no attribute row"
                )
            }
            Self::UnknownEntity { entity } => {
                write!(
                    f,
                    "entity {entity} has no attribute row to add to (a mod's type needs a declared set first)"
                )
            }
        }
    }
}
fn string(v: &Value) -> io::Result<&str> {
    v.as_str()
        .ok_or_else(|| invalid("expected attribute string"))
}
fn number(v: &Value) -> io::Result<f64> {
    let s = string(v)?;
    if s.len() != 16 {
        return Err(invalid("expected 16-digit attribute double bits"));
    }
    Ok(f64::from_bits(
        u64::from_str_radix(s, 16).map_err(|e| invalid(e.to_string()))?,
    ))
}
impl Catalog {
    pub(super) fn default_bases(&self, entity: &str) -> Option<Arc<[Option<f64>]>> {
        self.defaults.get(entity).cloned()
    }
    /// Imports a version-checked export of registered definitions and actual
    /// DefaultAttributes suppliers. Defaults contain base values, not instances.
    pub fn from_json(v: &Value) -> io::Result<Self> {
        if v["data_version"].as_i64() != Some(i64::from(DATA_VERSION)) {
            return Err(invalid("attribute catalog data version mismatch"));
        }
        let mut attributes = Vec::new();
        let mut ids = BTreeMap::new();
        for a in v["attributes"]
            .as_array()
            .ok_or_else(|| invalid("missing attributes"))?
        {
            let name = string(&a["id"])?;
            // Identifier normalization is shared with modifier IDs.
            let canonical = super::Modifier::new(name, 0.0, super::Operation::AddValue)?;
            if canonical.id() != name
                || ids
                    .insert(name.into(), AttributeId(attributes.len()))
                    .is_some()
            {
                return Err(invalid("noncanonical or duplicate attribute identifier"));
            }
            let default = number(&a["default"])?;
            let definition = if a.get("min").is_some() {
                Definition::ranged(default, number(&a["min"])?, number(&a["max"])?)?
            } else {
                Definition::basic(default)
            };
            attributes.push(RegisteredAttribute {
                name: name.into(),
                definition,
                syncable: a["syncable"]
                    .as_bool()
                    .ok_or_else(|| invalid("missing attribute sync flag"))?,
                // The vanilla export carries no tooltip style: vanilla's own
                // colors and formats stay where the tooltip reads them.
                tooltip: super::Tooltip::default(),
            });
        }
        let mut defaults = BTreeMap::new();
        for (entity, values) in v["entities"]
            .as_object()
            .ok_or_else(|| invalid("missing entity defaults"))?
        {
            let mut bases = vec![None; attributes.len()];
            for (name, base) in values
                .as_object()
                .ok_or_else(|| invalid("invalid entity supplier"))?
            {
                let id = ids
                    .get(name)
                    .ok_or_else(|| invalid("unknown supplier attribute"))?;
                bases[id.0] = Some(number(base)?);
            }
            defaults.insert(entity.clone(), Arc::from(bases));
        }
        Ok(Self {
            attributes,
            ids,
            defaults,
        })
    }
    /// The entity type's supplier base of an attribute, if the type has it.
    pub fn default_base(&self, entity: &str, id: AttributeId) -> Option<f64> {
        self.defaults.get(entity).and_then(|b| b[id.0])
    }
    /// The entity type's whole supplier row, attribute name to base — the
    /// shape of an `entity-types.json` row. None if the type has no row.
    pub fn entity_row(&self, entity: &str) -> Option<std::collections::HashMap<String, f64>> {
        let bases = self.defaults.get(entity)?;
        Some(
            self.attributes
                .iter()
                .enumerate()
                .filter_map(|(i, a)| bases[i].map(|base| (a.name.clone(), base)))
                .collect(),
        )
    }
    /// A copy of the catalog with `entity`'s supplier built from its
    /// `template`'s row and `overrides`: a mod entity type's declared
    /// attribute set (`EntityAttributeCreationEvent`), as its content row is
    /// the template's values with its own on top. An override id the catalog
    /// does not know, or a template without a row, refuses, naming the
    /// entity.
    pub fn with_entity_row(
        &self,
        entity: &str,
        template: &str,
        overrides: &[(String, f64)],
    ) -> Result<Catalog, EntityRowError> {
        let start = self
            .defaults
            .get(template)
            .ok_or_else(|| EntityRowError::UnknownTemplate {
                entity: entity.to_owned(),
                template: template.to_owned(),
            })?;
        self.with_bases(
            entity,
            start,
            overrides.iter().map(|(name, base)| (name, Some(*base))),
        )
    }
    /// A copy of the catalog with attributes added to `entity`'s own supplier
    /// (NeoForge's `EntityAttributeModificationEvent`, whose `add` puts an
    /// attribute on an existing type's `AttributeSupplier.Builder`): each
    /// named attribute takes its base, or the attribute's own default with
    /// none (the one-argument `add`), whether the row had it or not. The rest
    /// of the row stays. An attribute the catalog does not know, or an entity
    /// type without a row, refuses, naming the entity.
    pub fn with_entity_additions(
        &self,
        entity: &str,
        additions: &[(String, Option<f64>)],
    ) -> Result<Catalog, EntityRowError> {
        let start = self
            .defaults
            .get(entity)
            .ok_or_else(|| EntityRowError::UnknownEntity {
                entity: entity.to_owned(),
            })?;
        self.with_bases(
            entity,
            start,
            additions.iter().map(|(name, base)| (name, *base)),
        )
    }
    /// A copy of the catalog with new attribute types appended
    /// (`Registrar::declare_attribute`, arismoko/horizon#812): after every attribute the
    /// export listed, in order sorted by declaring mod id, then attribute id
    /// — the network order every side running the same mods builds, since
    /// [`AttributeId::registry_id`](Self::attribute)'s number is the index.
    /// Vanilla ids never shift. Every entity's default row grows to the new
    /// width, so a row resolved later (arismoko/horizon#845's `with_entity_row`, arismoko/horizon#846's
    /// additions) may name an attribute a mod added. A duplicate id — one
    /// the export already listed, or one two mods declared — refuses,
    /// naming the id.
    pub fn with_attributes(&self, added: &[AddedAttribute]) -> io::Result<Catalog> {
        let mut attributes = self.attributes.clone();
        let mut ids = self.ids.clone();
        let from = attributes.len();
        let mut sorted: Vec<&AddedAttribute> = added.iter().collect();
        sorted.sort_by(|a, b| {
            (a.mod_id.as_str(), a.id.as_str()).cmp(&(b.mod_id.as_str(), b.id.as_str()))
        });
        for (offset, attribute) in sorted.into_iter().enumerate() {
            let id = AttributeId(from + offset);
            if self.ids.contains_key(attribute.id.as_str())
                || ids.insert(attribute.id.clone(), id).is_some()
            {
                return Err(invalid(format!(
                    "duplicate attribute identifier {}",
                    attribute.id
                )));
            }
            attributes.push(RegisteredAttribute {
                name: attribute.id.clone(),
                definition: attribute.definition,
                syncable: attribute.syncable,
                tooltip: attribute.tooltip.clone(),
            });
        }
        let defaults = self
            .defaults
            .iter()
            .map(|(entity, bases)| {
                let mut grown = bases.to_vec();
                grown.resize(attributes.len(), None);
                (entity.clone(), Arc::from(grown))
            })
            .collect();
        Ok(Catalog {
            attributes,
            ids,
            defaults,
        })
    }
    /// A copy of the catalog with the bounds mods asked to widen
    /// (`Registrar::widen_attribute_bounds`, arismoko/horizon#3728): each attribute's bounds
    /// become the union of its own and every ask's, whatever their order, so
    /// an ask narrower than the result changes nothing. Defaults, ids and
    /// entity rows stay. An unknown attribute, or an ask whose range leaves
    /// the attribute's default out, refuses, naming the mod.
    pub fn with_widened_bounds(&self, asks: &[WidenedBounds]) -> Result<Catalog, WidenBoundsError> {
        let mut attributes = self.attributes.clone();
        for ask in asks {
            let Some(id) = self.ids.get(ask.id.as_str()) else {
                return Err(WidenBoundsError::UnknownAttribute {
                    mod_id: ask.mod_id.clone(),
                    id: ask.id.clone(),
                });
            };
            let definition = &mut attributes[id.0].definition;
            if !(ask.min..=ask.max).contains(&definition.default_value()) {
                return Err(WidenBoundsError::DefaultOutside {
                    mod_id: ask.mod_id.clone(),
                    id: ask.id.clone(),
                });
            }
            *definition = definition.widened(ask.min, ask.max);
        }
        Ok(Catalog {
            attributes,
            ids: self.ids.clone(),
            defaults: self.defaults.clone(),
        })
    }
    /// A copy of the catalog whose `entity` row is `start` with `overrides`
    /// on top; an override without a base takes the attribute's default.
    fn with_bases<'a>(
        &self,
        entity: &str,
        start: &[Option<f64>],
        overrides: impl Iterator<Item = (&'a String, Option<f64>)>,
    ) -> Result<Catalog, EntityRowError> {
        let mut bases = start.to_vec();
        for (name, base) in overrides {
            let Some(id) = self.ids.get(name.as_str()) else {
                return Err(EntityRowError::UnknownAttribute {
                    entity: entity.to_owned(),
                    attribute: name.clone(),
                });
            };
            let default = self.attributes[id.0].definition.default_value();
            bases[id.0] = Some(base.unwrap_or(default));
        }
        let mut defaults = self.defaults.clone();
        defaults.insert(entity.to_owned(), Arc::from(bases));
        Ok(Catalog {
            attributes: self.attributes.clone(),
            ids: self.ids.clone(),
            defaults,
        })
    }
    pub fn id(&self, name: &str) -> Option<AttributeId> {
        self.ids.get(name).copied()
    }
    pub fn attribute(&self, id: AttributeId) -> &RegisteredAttribute {
        &self.attributes[id.0]
    }
    /// The definition (default and bounds) of the attribute named `name`, if
    /// the catalog has it: what every clamp and check of its value reads.
    pub fn definition(&self, name: &str) -> Option<Definition> {
        self.id(name).map(|id| self.attributes[id.0].definition)
    }
    pub fn attributes(&self) -> impl Iterator<Item = (AttributeId, &RegisteredAttribute)> {
        self.attributes
            .iter()
            .enumerate()
            .map(|(i, a)| (AttributeId(i), a))
    }
    pub fn entities(&self) -> impl Iterator<Item = &str> {
        self.defaults.keys().map(String::as_str)
    }
    pub fn create(self: &Arc<Self>, entity: &str) -> Option<EntityAttributes> {
        let bases = self.defaults.get(entity)?.clone();
        let n = bases.len();
        Some(EntityAttributes {
            catalog: self.clone(),
            bases,
            instances: vec![None; n],
            updates: vec![false; n],
            sync: vec![false; n],
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookupError {
    MissingAttribute,
    MissingModifier,
    InstanceMissingModifier,
}
/// Membership in the native update/sync sets, with no JVM identity-hash order
/// promise. Clearing a flag acknowledges that particular pending update.
pub struct DirtyObserver<'a> {
    update: &'a mut bool,
    sync: &'a mut bool,
    syncable: bool,
}
impl Observer for DirtyObserver<'_> {
    type Error = Infallible;
    fn dirty(&mut self, _: &mut Instance) -> Result<(), Infallible> {
        *self.update = true;
        if self.syncable {
            *self.sync = true;
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct EntityAttributes {
    catalog: Arc<Catalog>,
    bases: Arc<[Option<f64>]>,
    instances: Vec<Option<Box<Instance>>>,
    updates: Vec<bool>,
    sync: Vec<bool>,
}
impl EntityAttributes {
    pub fn catalog(&self) -> &Arc<Catalog> {
        &self.catalog
    }
    pub fn has(&self, id: AttributeId) -> bool {
        self.bases[id.0].is_some()
    }
    pub fn instance(&self, id: AttributeId) -> Option<&Instance> {
        self.instances[id.0].as_deref()
    }
    /// Merely querying a default value must not instantiate or dirty it.
    pub fn value(&mut self, id: AttributeId) -> Result<f64, LookupError> {
        if let Some(a) = self.instances[id.0].as_deref_mut() {
            return Ok(a.value());
        }
        let base = self.bases[id.0].ok_or(LookupError::MissingAttribute)?;
        Ok(self.catalog.attribute(id).definition.sanitize(base))
    }
    pub fn base_value(&self, id: AttributeId) -> Result<f64, LookupError> {
        self.instances[id.0]
            .as_deref()
            .map(Instance::base_value)
            .or(self.bases[id.0])
            .ok_or(LookupError::MissingAttribute)
    }
    pub fn has_modifier(&self, id: AttributeId, modifier: &str) -> bool {
        self.instance(id)
            .and_then(|a| a.modifier(modifier))
            .is_some()
    }
    pub fn modifier_value(&self, id: AttributeId, modifier: &str) -> Result<f64, LookupError> {
        if let Some(a) = self.instance(id) {
            return a
                .modifier(modifier)
                .map(super::Modifier::amount)
                .ok_or(LookupError::InstanceMissingModifier);
        }
        if self.has(id) {
            Err(LookupError::MissingModifier)
        } else {
            Err(LookupError::MissingAttribute)
        }
    }
    /// Native getInstance copies the supplier on first access and immediately
    /// schedules an update, plus a sync when the definition is client-syncable.
    pub fn ensure(&mut self, id: AttributeId) -> bool {
        if self.instances[id.0].is_some() {
            return true;
        }
        let Some(base) = self.bases[id.0] else {
            return false;
        };
        let definition = self.catalog.attribute(id);
        let mut instance = Instance::new(definition.definition);
        let mut observer = DirtyObserver {
            update: &mut self.updates[id.0],
            sync: &mut self.sync[id.0],
            syncable: definition.syncable,
        };
        // Registered default suppliers have no modifiers. replaceFrom always
        // invalidates even when the supplier base equals the registered default.
        let mut source = Instance::new(definition.definition);
        source.base = base;
        instance.replace_from(&source, &mut observer).unwrap();
        self.instances[id.0] = Some(Box::new(instance));
        true
    }
    /// Mutations must use the supplied observer to preserve dirty memberships.
    pub fn edit<R>(
        &mut self,
        id: AttributeId,
        f: impl FnOnce(&mut Instance, &mut DirtyObserver<'_>) -> R,
    ) -> Option<R> {
        if !self.ensure(id) {
            return None;
        }
        let mut observer = DirtyObserver {
            update: &mut self.updates[id.0],
            sync: &mut self.sync[id.0],
            syncable: self.catalog.attribute(id).syncable,
        };
        Some(f(
            self.instances[id.0].as_deref_mut().unwrap(),
            &mut observer,
        ))
    }
    /// Reset uses the entity supplier's base, without creating a missing instance.
    pub fn reset_base(&mut self, id: AttributeId) -> bool {
        let Some(base) = self.bases[id.0] else {
            return false;
        };
        if self.instances[id.0].is_some() {
            self.edit(id, |a, o| a.set_base_value(base, o).unwrap());
        }
        true
    }
    pub fn needs_update(&self, id: AttributeId) -> bool {
        self.updates[id.0]
    }
    pub fn needs_sync(&self, id: AttributeId) -> bool {
        self.sync[id.0]
    }
    pub fn clear_update(&mut self, id: AttributeId) {
        self.updates[id.0] = false;
    }
    pub fn clear_sync(&mut self, id: AttributeId) {
        self.sync[id.0] = false;
    }
}

#[cfg(test)]
mod row_tests {
    use super::*;

    fn catalog() -> Catalog {
        let bits = |n: f64| format!("{:016x}", n.to_bits());
        Catalog::from_json(&serde_json::json!({
            "data_version": DATA_VERSION,
            "attributes": [
                {"id": "minecraft:max_health", "default": bits(20.0), "min": bits(1.0), "max": bits(1024.0), "syncable": true},
                {"id": "minecraft:armor", "default": bits(0.0), "min": bits(0.0), "max": bits(30.0), "syncable": true},
                {"id": "minecraft:luck", "default": bits(0.0), "min": bits(-1024.0), "max": bits(1024.0), "syncable": true},
                {"id": "minecraft:follow_range", "default": bits(32.0), "min": bits(0.0), "max": bits(2048.0), "syncable": false}
            ],
            "entities": {
                "minecraft:zombie": {"minecraft:max_health": bits(20.0), "minecraft:armor": bits(2.0)}
            }
        }))
        .unwrap()
    }

    /// arismoko/horizon#3728: asks to widen bounds join as their union, in either order; a
    /// narrower ask changes nothing; the default stays; an unknown attribute
    /// or a range without the default refuses, naming the mod.
    #[test]
    fn widened_bounds_are_the_union_of_every_ask() {
        let catalog = catalog();
        let ask = |mod_id: &str, id: &str, min: f64, max: f64| WidenedBounds {
            mod_id: mod_id.into(),
            id: id.into(),
            min,
            max,
        };
        let health = |c: &Catalog| c.definition("minecraft:max_health").unwrap();
        let small = ask("alpha", "minecraft:max_health", 1.0, 1e5);
        let large = ask("beta", "minecraft:max_health", 1.0, 1e6);
        let narrow = ask("gamma", "minecraft:max_health", 5.0, 100.0);
        for asks in [
            [small.clone(), large.clone(), narrow.clone()],
            [narrow.clone(), large.clone(), small.clone()],
        ] {
            let widened = catalog.with_widened_bounds(&asks).unwrap();
            assert_eq!(health(&widened).bounds(), Some((1.0, 1e6)));
            assert_eq!(health(&widened).default_value(), 20.0);
            assert_eq!(health(&widened).sanitize(5000.0), 5000.0);
        }
        assert_eq!(health(&catalog).sanitize(5000.0), 1024.0, "vanilla's stay");
        let lower = catalog
            .with_widened_bounds(&[ask("alpha", "minecraft:luck", -1e4, 0.0)])
            .unwrap();
        assert_eq!(
            lower.definition("minecraft:luck").unwrap().bounds(),
            Some((-1e4, 1024.0))
        );
        assert_eq!(
            catalog
                .with_widened_bounds(&[ask("alpha", "minecraft:mana", 0.0, 1.0)])
                .unwrap_err(),
            WidenBoundsError::UnknownAttribute {
                mod_id: "alpha".into(),
                id: "minecraft:mana".into()
            }
        );
        assert_eq!(
            catalog
                .with_widened_bounds(&[ask("alpha", "minecraft:follow_range", 40.0, 1e4)])
                .unwrap_err(),
            WidenBoundsError::DefaultOutside {
                mod_id: "alpha".into(),
                id: "minecraft:follow_range".into()
            }
        );
    }

    /// A declared row starts from its template's, as a content row does and
    /// `Zombie.createAttributes` does for the event's supplier (arismoko/horizon#845).
    #[test]
    fn a_declared_row_starts_from_its_templates() {
        let catalog = catalog();
        let merged = catalog
            .with_entity_row(
                "ruby:ghoul",
                "minecraft:zombie",
                &[("minecraft:max_health".into(), 55.0)],
            )
            .unwrap();
        let health = merged.id("minecraft:max_health").unwrap();
        let armor = merged.id("minecraft:armor").unwrap();
        assert_eq!(merged.default_base("ruby:ghoul", health), Some(55.0));
        // The template's own attributes come along.
        assert_eq!(merged.default_base("ruby:ghoul", armor), Some(2.0));
        // The catalog itself keeps what it had.
        assert_eq!(catalog.default_base("minecraft:zombie", health), Some(20.0));
        assert_eq!(catalog.default_base("ruby:ghoul", health), None);
    }

    #[test]
    fn a_declared_row_refuses_unknown_attributes_and_templates() {
        let catalog = catalog();
        assert!(matches!(
            catalog.with_entity_row(
                "ruby:ghoul",
                "minecraft:zombie",
                &[("missing:attribute".into(), 1.0)]
            ),
            Err(EntityRowError::UnknownAttribute { .. })
        ));
        assert!(matches!(
            catalog.with_entity_row("ruby:ghoul", "missing:template", &[]),
            Err(EntityRowError::UnknownTemplate { .. })
        ));
    }

    /// A merged row reads back whole, name to base, as an
    /// `entity-types.json` row is.
    #[test]
    fn an_entity_row_lists_its_values() {
        let catalog = catalog();
        let merged = catalog
            .with_entity_row(
                "ruby:ghoul",
                "minecraft:zombie",
                &[("minecraft:max_health".into(), 55.0)],
            )
            .unwrap();
        assert_eq!(
            merged.entity_row("ruby:ghoul").unwrap(),
            [
                ("minecraft:max_health".to_owned(), 55.0),
                ("minecraft:armor".to_owned(), 2.0)
            ]
            .into()
        );
        assert_eq!(merged.entity_row("missing:entity"), None);
    }

    /// Additions put attributes on a type's own row
    /// (`EntityAttributeModificationEvent.add`, arismoko/horizon#846): one it lacks joins the
    /// row, with its base or the attribute's default; one it has takes the
    /// new base; the rest of the row stays.
    #[test]
    fn additions_extend_a_types_own_row() {
        let catalog = catalog();
        let added = catalog
            .with_entity_additions(
                "minecraft:zombie",
                &[
                    ("minecraft:luck".into(), Some(3.5)),
                    ("minecraft:follow_range".into(), None),
                    ("minecraft:armor".into(), Some(4.0)),
                ],
            )
            .unwrap();
        assert_eq!(
            added.entity_row("minecraft:zombie").unwrap(),
            [
                ("minecraft:max_health".to_owned(), 20.0),
                ("minecraft:armor".to_owned(), 4.0),
                ("minecraft:luck".to_owned(), 3.5),
                ("minecraft:follow_range".to_owned(), 32.0),
            ]
            .into()
        );
        // A new map of the type has the added attribute.
        let mut live = Arc::new(added).create("minecraft:zombie").unwrap();
        let luck = catalog.id("minecraft:luck").unwrap();
        assert_eq!(live.value(luck).unwrap(), 3.5);
        // The catalog itself keeps what it had.
        assert_eq!(catalog.default_base("minecraft:zombie", luck), None);
    }

    /// Mod attributes append after every vanilla attribute, sorted by
    /// declaring mod id then attribute id (arismoko/horizon#812): the order is the network
    /// id's, the same whatever order the mods declared in, and vanilla ids
    /// never shift.
    #[test]
    fn mod_attributes_append_after_vanilla_in_a_sorted_order() {
        use super::super::Sentiment;
        let catalog = catalog();
        let added = |mod_id: &str, id: &str, syncable| AddedAttribute {
            mod_id: mod_id.into(),
            id: id.into(),
            definition: Definition::ranged(1.0, 0.0, 10.0).unwrap(),
            syncable,
            tooltip: super::super::Tooltip {
                sentiment: Sentiment::Neutral,
                ..Default::default()
            },
        };
        // Declared in the order the mods happen to register: the catalog
        // sorts them by mod id, then attribute id.
        let grown = catalog
            .with_attributes(&[
                added("zeta", "zeta:alpha", true),
                added("alpha", "alpha:zeta", false),
                added("alpha", "alpha:beta", true),
            ])
            .unwrap();
        for (name, id) in [
            ("minecraft:max_health", 0),
            ("minecraft:armor", 1),
            ("minecraft:luck", 2),
            ("minecraft:follow_range", 3),
            ("alpha:beta", 4),
            ("alpha:zeta", 5),
            ("zeta:alpha", 6),
        ] {
            assert_eq!(grown.id(name).unwrap().registry_id(), id, "{name}");
        }
        assert_eq!(
            grown.id("minecraft:max_health"),
            catalog.id("minecraft:max_health"),
            "vanilla ids never shift"
        );
        // The same declarations in another order build the same ids.
        let again = catalog
            .with_attributes(&[
                added("alpha", "alpha:zeta", false),
                added("alpha", "alpha:beta", true),
                added("zeta", "zeta:alpha", true),
            ])
            .unwrap();
        assert_eq!(again.id("zeta:alpha"), grown.id("zeta:alpha"));
        assert_eq!(again.id("alpha:beta"), grown.id("alpha:beta"));
        // Every entity's row grows to the new width.
        let mut live = Arc::new(grown.clone()).create("minecraft:zombie").unwrap();
        let added_id = grown.id("alpha:beta").unwrap();
        assert_eq!(live.value(added_id), Err(LookupError::MissingAttribute));
        // Grown to the new width, but the type does not gain the attribute:
        // an addition below may put it on the row, with its default.
        assert!(!live.has(added_id));
        let added_row = grown
            .with_entity_additions("minecraft:zombie", &[("alpha:beta".into(), None)])
            .unwrap();
        let mut live = Arc::new(added_row).create("minecraft:zombie").unwrap();
        assert_eq!(live.value(added_id), Ok(1.0));
        // A duplicate — a vanilla id, or one declared twice — refuses.
        assert!(
            catalog
                .with_attributes(&[added("minecraft", "minecraft:armor", true)])
                .is_err()
        );
        assert!(
            catalog
                .with_attributes(&[
                    added("alpha", "alpha:beta", true),
                    added("zeta", "alpha:beta", false)
                ])
                .is_err()
        );
    }

    #[test]
    fn additions_refuse_unknown_attributes_and_types_without_a_row() {
        let catalog = catalog();
        assert_eq!(
            catalog
                .with_entity_additions("minecraft:zombie", &[("missing:attribute".into(), None)])
                .err(),
            Some(EntityRowError::UnknownAttribute {
                entity: "minecraft:zombie".into(),
                attribute: "missing:attribute".into(),
            })
        );
        assert_eq!(
            catalog
                .with_entity_additions("minecraft:pig", &[("minecraft:luck".into(), None)])
                .err(),
            Some(EntityRowError::UnknownEntity {
                entity: "minecraft:pig".into(),
            })
        );
    }
}
