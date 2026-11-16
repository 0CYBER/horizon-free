//! Accepted native block entity membership, frozen before resources or worlds load.
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, OnceLock},
};

/// A block's factory and accepted entity types, frozen together at startup.
#[derive(Debug, PartialEq, Eq)]
struct BlockMembership {
    declared: Option<String>,
    template: Option<String>,
    factory_kind: Option<String>,
    valid: BTreeSet<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct BlockEntityTypes {
    blocks: BTreeMap<String, BlockMembership>,
    /// Names in protocol order: a reload may not change numeric wire IDs either.
    wire: Vec<Arc<str>>,
    keys: BTreeSet<Arc<str>>,
    /// Each registered name's numeric wire ID, its place in `wire`.
    numeric_ids: HashMap<Arc<str>, u32>,
}

impl BlockEntityTypes {
    /// Membership has already been checked by the content plan.
    pub fn new(blocks: BTreeMap<String, String>, wire: Vec<String>) -> Self {
        let wire: Vec<Arc<str>> = wire.into_iter().map(Into::into).collect();
        let keys = wire.iter().filter(|id| !id.is_empty()).cloned().collect();
        let numeric_ids = (0..)
            .zip(&wire)
            .filter(|(_, id)| !id.is_empty())
            .map(|(numeric, id)| (id.clone(), numeric))
            .collect();
        let blocks = blocks
            .into_iter()
            .map(|(block, kind)| {
                (
                    block,
                    BlockMembership {
                        declared: Some(kind.clone()),
                        template: None,
                        factory_kind: Some(kind.clone()),
                        valid: [kind].into(),
                    },
                )
            })
            .collect();
        Self {
            blocks,
            wire,
            keys,
            numeric_ids,
        }
    }

    /// The declared native factory; added validity never changes this selection.
    pub fn type_for(&self, block: &str) -> Option<&str> {
        self.blocks.get(block)?.declared.as_deref()
    }

    /// Record a vanilla factory inherited from a content block's template.
    pub fn template(&mut self, block: String, template: String, kind: String) {
        let valid = if block == template
            || matches!(kind.as_str(), "minecraft:sign" | "minecraft:hanging_sign")
        {
            [kind.clone()].into()
        } else {
            BTreeSet::new()
        };
        self.blocks.entry(block).or_insert_with(|| BlockMembership {
            declared: None,
            template: Some(template),
            factory_kind: Some(kind),
            valid,
        });
    }

    /// Extend validity without changing the block's factory.
    pub fn add_valid(&mut self, block: &str, kind: &str) -> Result<(), &'static str> {
        if !self.keys.contains(kind) {
            return Err("unknown block entity type");
        }
        self.blocks
            .entry(block.to_owned())
            .or_insert_with(|| BlockMembership {
                declared: None,
                template: None,
                factory_kind: None,
                valid: BTreeSet::new(),
            })
            .valid
            .insert(kind.to_owned());
        Ok(())
    }

    /// Explicit membership, including a negative result for known entity blocks.
    pub fn valid(&self, block: &str, kind: &str) -> Option<bool> {
        Some(self.blocks.get(block)?.valid.contains(kind))
    }

    /// Whether a type beyond the selected factory has been accepted.
    pub fn additional_valid(&self, block: &str, kind: &str) -> bool {
        self.blocks.get(block).is_some_and(|entry| {
            entry.factory_kind.as_deref() != Some(kind) && entry.valid.contains(kind)
        })
    }

    /// The original vanilla template, independent of added valid types.
    pub fn template_for(&self, block: &str) -> Option<&str> {
        self.blocks.get(block)?.template.as_deref()
    }

    /// Whether the selected factory's type is valid for this block.
    pub fn factory_valid(&self, block: &str) -> Option<bool> {
        let entry = self.blocks.get(block)?;
        Some(entry.valid.contains(entry.factory_kind.as_deref()?))
    }

    /// Deterministic validity entries for connection identity and startup validation.
    pub fn memberships(&self) -> impl Iterator<Item = (&str, &str)> {
        self.blocks.iter().flat_map(|(block, entry)| {
            entry
                .valid
                .iter()
                .map(move |kind| (block.as_str(), kind.as_str()))
        })
    }

    /// Memberships that contribute beyond the built-in vanilla table.
    pub fn identity_memberships(&self) -> impl Iterator<Item = (&str, &str)> {
        self.blocks.iter().flat_map(|(block, entry)| {
            entry
                .valid
                .iter()
                .filter(move |kind| {
                    !block.starts_with("minecraft:") || entry.factory_kind.as_ref() != Some(*kind)
                })
                .map(move |kind| (block.as_str(), kind.as_str()))
        })
    }

    /// Registered names in protocol order, excluding empty slots.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.wire
            .iter()
            .filter(|id| !id.is_empty())
            .map(AsRef::as_ref)
    }

    /// A canonical registered identity, shared without allocating at lookup.
    pub fn key(&self, id: &str) -> Option<Arc<str>> {
        self.keys.get(id).cloned()
    }

    /// The numeric wire ID of the registered type `id`: a small integer that names it for this
    /// run, below [`Self::numeric_id_bound`].
    pub fn numeric_id(&self, id: &str) -> Option<u32> {
        self.numeric_ids.get(id).copied()
    }

    /// One more than the largest numeric wire ID.
    pub fn numeric_id_bound(&self) -> usize {
        self.wire.len()
    }

    /// Whether a mod declared the block entity type `id`.
    /// Wire keys include vanilla types; `mod_content::Plan::new` refuses native
    /// declarations in `minecraft:`.
    pub fn declares(&self, id: &str) -> bool {
        !id.starts_with("minecraft:") && self.keys.contains(id)
    }
}

static TYPES: OnceLock<BlockEntityTypes> = OnceLock::new();

/// The declared native factory. Reading before installation never freezes an empty table.
pub fn type_for(block: &str) -> Option<&'static str> {
    current()?.type_for(block)
}

/// Explicit validity when this block participates in the canonical table.
pub fn valid(block: &str, kind: &str) -> Option<bool> {
    current()?.valid(block, kind)
}

/// Whether an additional type has been accepted beyond the original factory.
pub fn additional_valid(block: &str, kind: &str) -> bool {
    current().is_some_and(|types| types.additional_valid(block, kind))
}

/// Whether the block's own factory is accepted by the canonical validity set.
pub fn factory_valid(block: &str) -> Option<bool> {
    current()?.factory_valid(block)
}

/// The vanilla factory template of a content block, or its own name.
pub fn factory_block(block: &str) -> &str {
    current()
        .and_then(|types| types.template_for(block))
        .unwrap_or(block)
}

pub fn current() -> Option<&'static BlockEntityTypes> {
    TYPES.get()
}

/// The numeric wire ID of the registered type `id` (none before startup installs the table).
pub fn numeric_id(id: &str) -> Option<u32> {
    current()?.numeric_id(id)
}

/// Whether a mod declared the block entity type `id` (none before startup
/// installs the table).
pub fn declared(id: &str) -> bool {
    current().is_some_and(|types| types.declares(id))
}

/// An identical setup is harmless; changing membership or wire IDs needs a restart.
pub fn install(types: BlockEntityTypes) -> Result<(), &'static str> {
    freeze(&TYPES, types)
}

fn freeze(slot: &OnceLock<BlockEntityTypes>, types: BlockEntityTypes) -> Result<(), &'static str> {
    match slot.set(types) {
        Ok(()) => Ok(()),
        Err(types) if slot.get() == Some(&types) => Ok(()),
        Err(_) => Err("block entity types changed; restart the game to use them"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn many_to_many_validity_keeps_factories_and_freezes_additions() {
        let mut types = BlockEntityTypes::new(
            [("mod:machine".into(), "mod:own".into())].into(),
            vec![
                "minecraft:vault".into(),
                "mod:own".into(),
                "mod:other".into(),
            ],
        );
        types.template(
            "mod:vault".into(),
            "minecraft:vault".into(),
            "minecraft:vault".into(),
        );
        assert_eq!(types.factory_valid("mod:vault"), Some(false));
        types.add_valid("mod:vault", "minecraft:vault").unwrap();
        types.add_valid("mod:vault", "mod:other").unwrap();
        types.add_valid("mod:machine", "mod:other").unwrap();
        types.add_valid("mod:vault", "mod:other").unwrap();
        assert_eq!(types.type_for("mod:machine"), Some("mod:own"));
        assert_eq!(types.type_for("mod:vault"), None);
        assert_eq!(types.template_for("mod:vault"), Some("minecraft:vault"));
        assert_eq!(types.factory_valid("mod:vault"), Some(true));
        assert_eq!(types.valid("mod:machine", "mod:own"), Some(true));
        assert_eq!(types.valid("mod:machine", "mod:other"), Some(true));
        assert_eq!(types.valid("mod:vault", "minecraft:vault"), Some(true));
        assert_eq!(types.valid("mod:vault", "mod:other"), Some(true));
        assert_eq!(types.valid("mod:vault", "mod:own"), Some(false));
        assert_eq!(types.memberships().count(), 4);
        assert_eq!(
            types.add_valid("mod:vault", "missing:type"),
            Err("unknown block entity type")
        );
        assert_eq!(types.memberships().count(), 4);
        let slot = OnceLock::new();
        freeze(&slot, types).unwrap();
        assert!(freeze(&slot, BlockEntityTypes::new(BTreeMap::new(), Vec::new())).is_err());
    }

    #[test]
    fn declaration_lookup_keeps_vanilla_types_and_block_ids_out() {
        let types = BlockEntityTypes::new(
            [
                ("test:press".into(), "test:machine".into()),
                ("test:mill".into(), "test:machine".into()),
                ("test:casing".into(), "test:single".into()),
            ]
            .into(),
            vec![
                "minecraft:chest".into(),
                "test:machine".into(),
                "test:single".into(),
                "test:empty".into(),
            ],
        );
        assert_eq!(types.numeric_id("minecraft:chest"), Some(0));
        assert_eq!(types.numeric_id("test:empty"), Some(3));
        assert_eq!(types.numeric_id("test:press"), None);
        assert_eq!(types.numeric_id(""), None);
        for (id, declared) in [
            ("test:machine", true),
            ("test:single", true),
            ("test:empty", true),
            ("minecraft:chest", false),
            ("test:press", false),
            ("test:mill", false),
            ("test:casing", false),
            ("other:machine", false),
            ("", false),
        ] {
            assert_eq!(types.declares(id), declared, "{id}");
        }
    }

    #[test]
    fn shared_membership_and_wire_ids_are_frozen_together() {
        let slot = OnceLock::new();
        let types = || {
            BlockEntityTypes::new(
                [
                    ("test:press".into(), "test:machine".into()),
                    ("test:mill".into(), "test:machine".into()),
                ]
                .into(),
                vec!["minecraft:sign".into(), "test:machine".into()],
            )
        };
        freeze(&slot, types()).unwrap();
        freeze(&slot, types()).unwrap();
        let accepted = slot.get().unwrap();
        for block in ["test:press", "test:mill"] {
            assert_eq!(accepted.type_for(block), Some("test:machine"));
        }
        for block in ["test:casing", "other:press", "minecraft:stone"] {
            assert_eq!(accepted.type_for(block), None);
        }
        let mut changed = types();
        changed.blocks.remove("test:mill");
        assert!(freeze(&slot, changed).is_err());
        let mut changed = types();
        changed.wire.swap(0, 1);
        assert!(freeze(&slot, changed).is_err());
        assert_eq!(slot.get(), Some(&types()));
    }
}
