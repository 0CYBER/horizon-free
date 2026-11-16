//! Engine-owned mod block entity storage and revision tracking.
use horizon_core::nbt::Compound;
#[cfg(test)]
use horizon_core::nbt::Tag;
use horizon_items::items::ItemStack;
/// A mod's block entity, read and changed through its fields
/// (`ModBlockEntityFields`) as a struct's: `entity.nbt`. Each change goes
/// through `DerefMut` and gives it a new `revision`, so a client tells
/// whether what it kept of it (`BlockEntityDraw::keep`) is still right
/// without comparing the whole entity, and no write can skip that. Its type
/// identity is fixed at construction and is not part of the mutable fields.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ModBlockEntity {
    pub(crate) id: String,
    /// `id`'s numeric wire ID (`block_entity_types::numeric_id`), looked up once when the entity is
    /// made, so a published loop's slot compares types by integer; `None` for an id the table
    /// does not have, or one made before the table is installed. The table is frozen for the
    /// process, so the ID never goes stale.
    pub(crate) numeric_id: Option<u32>,
    pub(crate) fields: ModBlockEntityFields,
    pub(crate) revision: ModRevision,
}
/// What a `ModBlockEntity` can change: its saved compound without `id`,
/// `x`, `y`, `z` and `Items`, its item slots
/// (`Items`) when its block has a container (`ModContainer`), and the data
/// slots its menu shows (`ContainerData`: a furnace menu's flame and arrow),
/// which the mod sets and nothing saves.
#[derive(Clone, Debug, Default, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ModBlockEntityFields {
    pub nbt: Compound,
    pub items: Vec<Option<ItemStack>>,
    pub data: Vec<i32>,
}
/// Which contents a `ModBlockEntity` has had: one per entity made (a clone
/// is another), counted on by each change. Equal revisions, equal contents.
/// Neither saved nor compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ModRevision {
    pub(crate) born: u64,
    pub(crate) changes: u64,
}

impl ModRevision {
    /// The unique construction identity, copied into API revision snapshots.
    pub fn identity(self) -> u64 {
        self.born
    }
    /// The number of field writes since construction.
    pub fn changes(self) -> u64 {
        self.changes
    }
}
impl ModBlockEntity {
    /// Apply a validated callback snapshot in place, keeping this entity's identity.
    /// The game passes the snapshot's original write count, including no-op writes.
    pub fn apply_snapshot(&mut self, fields: ModBlockEntityFields, changes: u64) {
        self.fields = fields;
        self.revision.changes = changes;
    }

    pub fn new(id: String, fields: ModBlockEntityFields) -> Self {
        let numeric_id = horizon_client_core::block_entity_types::numeric_id(&id);
        Self::with_numeric_id(id, numeric_id, fields)
    }
    fn with_numeric_id(id: String, numeric_id: Option<u32>, fields: ModBlockEntityFields) -> Self {
        Self {
            id,
            numeric_id,
            fields,
            revision: ModRevision {
                born: horizon_mod_runtime::block_entities::new_identity(),
                changes: 0,
            },
        }
    }
    /// The block entity's immutable type id (`mymod:machine`).
    ///
    /// ```compile_fail
    /// let mut entity = horizon_client_render::containers::ModBlockEntity::default();
    /// entity.id = "other:machine".into();
    /// ```
    ///
    /// ```compile_fail
    /// let mut entity = horizon_client_render::containers::ModBlockEntity::default();
    /// entity.id().push_str("_changed");
    /// ```
    pub fn id(&self) -> &str {
        &self.id
    }
    /// The type's numeric wire ID, resolved from [`Self::id`] when the entity was made; `None`
    /// for a type the game has not registered.
    pub fn numeric_id(&self) -> Option<u32> {
        self.numeric_id
    }
    pub fn revision(&self) -> ModRevision {
        self.revision
    }
    /// A client's copy loads the tag the server sent (`loadWithComponents`
    /// in `ClientPacketListener.handleBlockEntityData`): `tag`'s fields
    /// replace its own of the same names. One it already has as sent is
    /// left alone, so a tag sent again unchanged keeps its revision.
    pub fn merge_update_tag(&mut self, tag: &Compound) {
        for (k, v) in tag {
            if self.nbt.get(k) != Some(v) {
                self.nbt.insert(k.clone(), v.clone());
            }
        }
    }
    /// Queue the one-shot fields the next block change sends, in place of any
    /// waiting; an empty batch clears them. The runtime keeps them under this
    /// entity's identity, so a save, a clone or a comparison never sees them
    /// and the revision stays.
    pub fn set_client_update_fields(&self, fields: Compound) {
        horizon_mod_runtime::block_entities::set_client_update_fields(self.revision.born, fields);
    }
    /// Take the queued one-shot fields for a tag about to go to every viewer.
    pub fn take_client_update_fields(&self) -> Option<Compound> {
        horizon_mod_runtime::block_entities::take_client_update_fields(self.revision.born)
    }
    /// Whether one-shot fields wait for a block change.
    pub fn has_client_update_fields(&self) -> bool {
        horizon_mod_runtime::block_entities::has_client_update_fields(self.revision.born)
    }
}
/// A removed or unloaded entity's queued fields go with it.
impl Drop for ModBlockEntity {
    fn drop(&mut self) {
        drop(self.take_client_update_fields());
    }
}
/// The empty placeholder a tick leaves in a mod block entity's place: no type, nothing looked up.
impl Default for ModBlockEntity {
    fn default() -> Self {
        Self::with_numeric_id(String::new(), None, ModBlockEntityFields::default())
    }
}
impl Clone for ModBlockEntity {
    fn clone(&self) -> Self {
        Self::with_numeric_id(self.id.clone(), self.numeric_id, self.fields.clone())
    }
}
impl PartialEq for ModBlockEntity {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.fields == other.fields
    }
}
impl std::fmt::Debug for ModBlockEntity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModBlockEntity")
            .field("id", &self.id)
            .field("fields", &self.fields)
            .finish()
    }
}
impl std::ops::Deref for ModBlockEntity {
    type Target = ModBlockEntityFields;
    fn deref(&self) -> &ModBlockEntityFields {
        &self.fields
    }
}
impl std::ops::DerefMut for ModBlockEntity {
    fn deref_mut(&mut self) -> &mut ModBlockEntityFields {
        self.revision.changes += 1;
        &mut self.fields
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mod_block_entitys_revision_moves_with_each_change_and_only_then() {
        let mut machine = ModBlockEntity::new(
            "mymod:machine".into(),
            ModBlockEntityFields {
                data: vec![0],
                ..Default::default()
            },
        );
        let made = machine.revision();
        // Reads leave it.
        assert_eq!(machine.id(), "mymod:machine");
        assert_eq!(machine.data[0] + machine.nbt.len() as i32, 0);
        assert_eq!(machine.revision(), made);
        // Every way to change a field moves it.
        let mut seen = vec![made];
        let changes: [fn(&mut ModBlockEntity); 4] = [
            |m| m.data[0] = 1,
            |m| _ = m.nbt.insert("progress".into(), Tag::Int(3)),
            |m| m.items.push(None),
            |m| **m = ModBlockEntityFields::default(),
        ];
        for change in changes {
            change(&mut machine);
            assert_eq!(machine.id(), "mymod:machine");
            assert!(!seen.contains(&machine.revision()));
            seen.push(machine.revision());
        }
        // Another entity, or a clone, never shares one, even with equal
        // contents; equality is the contents' alone.
        let copy = machine.clone();
        assert_eq!(copy, machine);
        assert!(!seen.contains(&copy.revision()));
        let other_kind = ModBlockEntity::new("mymod:other".into(), machine.fields.clone());
        assert_ne!(other_kind, machine);
        let fresh = ModBlockEntity::default();
        assert_ne!(fresh.revision(), ModBlockEntity::default().revision());
    }

    #[test]
    fn queued_client_update_fields_stay_out_of_clones_and_go_with_their_entity() {
        let machine = ModBlockEntity::new("mymod:machine".into(), Default::default());
        let made = machine.revision();
        let batch = Compound::from([("effect".into(), Tag::Int(23))]);
        machine.set_client_update_fields(batch.clone());
        assert_eq!(machine.revision(), made);
        let copy = machine.clone();
        assert_eq!(copy, machine);
        assert!(!copy.has_client_update_fields());
        drop(copy);
        assert!(machine.has_client_update_fields());
        assert_eq!(machine.take_client_update_fields(), Some(batch.clone()));
        assert_eq!(machine.take_client_update_fields(), None);
        machine.set_client_update_fields(batch);
        drop(machine);
        assert_eq!(
            horizon_mod_runtime::block_entities::take_client_update_fields(made.identity()),
            None
        );
    }

    #[test]
    fn an_update_tag_sent_again_unchanged_keeps_the_revision() {
        let mut machine = ModBlockEntity::default();
        let tag = Compound::from([
            ("progress".into(), Tag::Int(3)),
            ("fuel".into(), Tag::Int(9)),
        ]);
        machine.merge_update_tag(&tag);
        let merged = machine.revision();
        machine.merge_update_tag(&tag);
        assert_eq!(machine.revision(), merged);
        // One field changed: replaced, the other kept, the revision moved.
        machine.merge_update_tag(&Compound::from([("progress".into(), Tag::Int(4))]));
        assert_ne!(machine.revision(), merged);
        assert_eq!(
            machine.nbt,
            Compound::from([
                ("progress".into(), Tag::Int(4)),
                ("fuel".into(), Tag::Int(9)),
            ])
        );
    }
}
