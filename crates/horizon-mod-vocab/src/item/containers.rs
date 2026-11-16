//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
/// A mod's block entity, read and changed through its fields
/// (`ModBlockEntityFields`) as a struct's: `entity.nbt`. Each change goes
/// through `DerefMut` and gives it a new `revision`, so a client tells
/// whether what it kept of it (`BlockEntityDraw::keep`) is still right
/// without comparing the whole entity, and no write can skip that. Its type
/// identity is fixed at construction and is not part of the mutable fields.
/// Returned guest snapshots apply only changed mutable fields through the same revision path.
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: transfers a flattened ModBlockEntitySnapshot, including revision counters and queued client fields, rather than internal fields storage.
pub struct ModBlockEntity {
    pub(crate) id: String,
    pub(crate) fields: ModBlockEntityFields,
    pub(crate) revision: ModRevision,
    /// What a callback asked for with `ModBlockEntityView::set_client_update_fields`, on its
    /// way to the runtime's queue. The queue itself never reaches a snapshot.
    pub(crate) client_update_fields: Option<Compound>,
}
/// What a `ModBlockEntity` can change: its saved compound without `id`,
/// `x`, `y`, `z` and `Items`, its item slots
/// (`Items`) when its block has a container (`ModContainer`), and the data
/// slots its menu shows (`ContainerData`: a furnace menu's flame and arrow),
/// which the mod sets and nothing saves.
#[derive(Clone, Debug, Default, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ModBlockEntityFields {
    pub nbt: Compound,
    pub items: Vec<Option<ItemStack>>,
    pub data: Vec<i32>,
}
/// Which contents a `ModBlockEntity` has had: one per entity made (a clone
/// is another), counted on by each change. Equal revisions, equal contents.
/// Rejected callback writebacks also advance the engine revision to invalidate stale copies;
/// this may cause one extra client redraw even though the fields were retained.
/// Neither saved nor compared.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ModRevision {
    pub(crate) born: u64,
    pub(crate) changes: u64,
}
// exhaustive: the game builds the snapshot; mods only read it
// hidden: the game's snapshot of a mod block entity, read by generated boundary code
/// An entity's transport form; identity and revision are read-only on writeback.
#[doc(hidden)]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ModBlockEntitySnapshot {
    pub(crate) id: String,
    pub(crate) fields: ModBlockEntityFields,
    pub(crate) revision: ModRevision,
    pub(crate) client_update_fields: Option<Compound>,
}
impl From<&ModBlockEntity> for ModBlockEntitySnapshot {
    fn from(entity: &ModBlockEntity) -> Self {
        Self {
            id: entity.id.clone(),
            fields: entity.fields.clone(),
            revision: entity.revision,
            client_update_fields: entity.client_update_fields.clone(),
        }
    }
}
impl From<ModBlockEntitySnapshot> for ModBlockEntity {
    fn from(snapshot: ModBlockEntitySnapshot) -> Self {
        Self {
            id: snapshot.id,
            fields: snapshot.fields,
            revision: snapshot.revision,
            client_update_fields: snapshot.client_update_fields,
        }
    }
}
impl ModBlockEntity {
    /// Apply returned mutable state through the native revision path, never its identity.
    pub(crate) fn apply_snapshot(&mut self, snapshot: ModBlockEntitySnapshot) {
        if self.fields != snapshot.fields {
            *std::ops::DerefMut::deref_mut(self) = snapshot.fields;
        }
        self.client_update_fields = snapshot.client_update_fields;
    }
}

impl ModRevision {
    // hidden: the game verifies callback snapshots still name their engine entity
    #[doc(hidden)]
    pub fn identity(self) -> u64 {
        self.born
    }

    // hidden: game write-back retains the callback's revision, not a new single write
    #[doc(hidden)]
    pub fn changes(self) -> u64 {
        self.changes
    }

    // hidden: copy the engine's revision data at the game boundary
    #[doc(hidden)]
    pub fn from_snapshot(born: u64, changes: u64) -> Self {
        Self { born, changes }
    }
}
impl ModBlockEntity {
    // hidden: game snapshot construction, retaining the engine's revision identity
    #[doc(hidden)]
    pub fn from_snapshot(id: String, fields: ModBlockEntityFields, revision: ModRevision) -> Self {
        Self {
            id,
            fields,
            revision,
            client_update_fields: None,
        }
    }

    // hidden: the game moves what a callback asked for to the runtime's queue
    #[doc(hidden)]
    pub fn client_update_fields(&self) -> Option<&Compound> {
        self.client_update_fields.as_ref()
    }

    pub fn new(id: String, fields: ModBlockEntityFields) -> Self {
        Self {
            id,
            fields,
            revision: ModRevision {
                born: horizon_mod_runtime::block_entities::new_identity(),
                changes: 0,
            },
            client_update_fields: None,
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
}
impl Default for ModBlockEntity {
    fn default() -> Self {
        Self::new(String::new(), ModBlockEntityFields::default())
    }
}
impl Clone for ModBlockEntity {
    fn clone(&self) -> Self {
        Self::new(self.id.clone(), self.fields.clone())
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
/// The container of a mod block's block entity, as a `WorldlyContainer`
/// says it: its slot count, the slots each face reaches
/// (`getSlotsForFace`, by `Direction`: down, up, north, south, west, east),
/// and the slots hoppers may put into (`canPlaceItemThroughFace`) and take
/// from (`canTakeItemThroughFace`); and the vanilla menu that shows its
/// slots when the block is used, if any (`MOD_MENUS`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ModContainer {
    pub slots: usize,
    pub faces: [Vec<usize>; 6],
    pub inputs: Vec<usize>,
    pub outputs: Vec<usize>,
    pub menu: Option<String>,
}
/// The fluid tanks of a mod block's block entity, in the container's shape:
/// each tank's capacity in millibuckets, the tanks each face reaches, and
/// the tanks a fill may reach and a drain may take. The tanks' contents are
/// not here: they live in the entity's saved compound, under `fluids`, so
/// they survive restarts and the block's own tick reads them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ModFluidTanks {
    /// Each tank's capacity in millibuckets, in tank order.
    pub tanks: Vec<i32>,
    pub faces: [Vec<usize>; 6],
    /// The tanks a fill may reach (`canPlaceItemThroughFace`).
    pub inputs: Vec<usize>,
    /// The tanks a drain may take from (`canTakeItemThroughFace`).
    pub outputs: Vec<usize>,
}
/// The energy buffer of a mod block's block entity (NeoForge's
/// `SimpleEnergyHandler`): its capacity, the most one insert and one extract
/// moves, and the faces a fill may come through and a drain may leave by.
/// The amount is not here: it lives in the entity's saved compound, under
/// `energy`, so it survives restarts and the block's own tick reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ModEnergyBuffer {
    pub capacity: i32,
    /// The most one insert moves; 0 takes none.
    pub max_insert: i32,
    /// The most one extract moves; 0 gives none.
    pub max_extract: i32,
    /// The faces, by `Direction` number, a fill may come through.
    pub insert_faces: [bool; 6],
    /// The faces a drain may leave by.
    pub extract_faces: [bool; 6],
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
    fn snapshot_preserves_negative_zero_writes() {
        for (positive, negative) in [
            (
                Tag::Float(0.0f32.to_bits()),
                Tag::Float((-0.0f32).to_bits()),
            ),
            (
                Tag::Double(0.0f64.to_bits()),
                Tag::Double((-0.0f64).to_bits()),
            ),
        ] {
            let mut entity = ModBlockEntity::default();
            entity.nbt.insert("value".into(), positive);
            let before = entity.revision();
            let mut snapshot = ModBlockEntitySnapshot::from(&entity);
            snapshot.fields.nbt.insert("value".into(), negative.clone());
            entity.apply_snapshot(snapshot);
            assert_eq!(entity.nbt.get(&"value".into()), Some(&negative));
            assert_ne!(entity.revision(), before);
        }
    }

    #[test]
    fn unchanged_nan_snapshots_do_not_advance_revision() {
        let mut entity = ModBlockEntity::default();
        entity.nbt.insert("float".into(), Tag::Float(0x7fc0_0123));
        entity
            .nbt
            .insert("double".into(), Tag::Double(0x7ff8_0000_0000_0123));
        let before = entity.revision();
        for _ in 0..20 {
            entity.apply_snapshot(ModBlockEntitySnapshot::from(&entity));
            assert_eq!(entity.revision(), before);
        }
        let mut snapshot = ModBlockEntitySnapshot::from(&entity);
        snapshot
            .fields
            .nbt
            .insert("float".into(), Tag::Float(0x7fc0_0124));
        entity.apply_snapshot(snapshot);
        assert_ne!(entity.revision(), before);
    }

    #[test]
    fn returned_snapshot_changes_only_fields_and_keeps_native_revision_bookkeeping() {
        let mut entity = ModBlockEntity::new("test:machine".into(), Default::default());
        let revision = entity.revision();
        let mut snapshot = ModBlockEntitySnapshot::from(&entity);
        snapshot.id = "forged:identity".into();
        snapshot.revision = ModBlockEntity::default().revision();
        entity.apply_snapshot(snapshot);
        assert_eq!(entity.id(), "test:machine");
        assert_eq!(entity.revision(), revision);
        let mut snapshot = ModBlockEntitySnapshot::from(&entity);
        snapshot.fields.data = vec![17];
        entity.apply_snapshot(snapshot);
        assert_eq!(entity.data, [17]);
        assert_eq!(entity.id(), "test:machine");
        assert_ne!(entity.revision(), revision);
        let restored = ModBlockEntity::from(ModBlockEntitySnapshot::from(&entity));
        assert_eq!(restored.revision(), entity.revision());
    }

    #[test]
    fn client_update_fields_come_back_with_a_snapshot_and_change_nothing_else() {
        let mut entity = ModBlockEntity::new("test:machine".into(), Default::default());
        let revision = entity.revision();
        let batch = Compound::from([("effect".into(), Tag::Int(23))]);
        crate::ModBlockEntityView::new(&mut entity).set_client_update_fields(batch.clone());
        assert_eq!(entity.revision(), revision);
        assert_eq!(entity.clone().client_update_fields(), None);
        let snapshot = ModBlockEntitySnapshot::from(&entity);
        let mut returned = ModBlockEntity::from_snapshot(
            entity.id().to_owned(),
            ModBlockEntityFields::default(),
            revision,
        );
        returned.apply_snapshot(snapshot);
        assert_eq!(returned.client_update_fields(), Some(&batch));
        assert_eq!(returned.revision(), revision);
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
