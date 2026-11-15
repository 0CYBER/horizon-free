//! Typed state a block entity Part keeps on each block entity of its type (arismoko/horizon#781, arismoko/horizon#796).
//!
//! The value lives in the block entity's own attachment map, the store every other
//! attachment uses: it saves with the chunk through the type's codec, is kept by
//! transactions, and goes with the block entity when it is removed, replaced or
//! unloaded. The Part object is shared behaviour and owns no instance's value.

use crate::*;
use std::{cell::RefCell, fmt, sync::Arc};

/// A block entity Part's typed state: one value per block entity of the Part's type,
/// owned by the game, not by the Part or a mod's statics.
///
/// Implement it beside [`BlockEntityPart`] and attach the Part with
/// [`Registrar::block_entity_part_with_state`](crate::Registrar). `BlockEntityPart`
/// itself stays object-safe: this trait is only for the typed calls below.
///
/// ```rust,ignore
/// struct Press;
/// impl BlockEntityPartState for Press {
///     type Data = u32;
///     const DATA: &'static AttachmentType<u32> = &PROGRESS;
/// }
/// // In a hook with `host: &mut dyn ServerHost` and the block entity's `entity`:
/// let done = Press::edit_state(host, entity, |_, progress| {
///     *progress += 1;
///     *progress >= 200
/// })?;
/// ```
///
/// Reads take `&dyn Host` and return a snapshot. Edits take `&mut dyn ServerHost`, run in
/// their own nested transaction and store the new value only when the closure returns.
/// Every call first checks that `entity` is still the block entity it named, in the
/// level this host is in: a replaced, removed or unloaded one refuses with
/// [`PartStateError::Stale`]. A block entity out of the world for its own tick still
/// answers, through [`BlockEntitiesHost::block_entity_identity`].
///
/// Server-side only for now: `DATA` must not sync, and a client's copy holds nothing
/// of it, so a client reads the default.
pub trait BlockEntityPartState: BlockEntityPart + Sized {
    /// The value each block entity of the Part's type keeps.
    type Data: Clone + Send + Sync + crate::wire::Wire + 'static;
    /// Its attachment key, default and codec. The key is in a namespace the mod owns;
    /// without a codec the value is kept for the session only (and a wasm mod needs one).
    const DATA: &'static AttachmentType<Self::Data>;

    /// The block entity's value, or the default when it has none. Inserts nothing.
    fn state(host: &dyn Host, entity: BlockEntityRef) -> Result<Self::Data, PartStateError> {
        Ok(Self::state_existing(host, entity)?.unwrap_or_else(Self::DATA.default))
    }

    /// The block entity's value, or `None` when it has none. Inserts nothing.
    fn state_existing(
        host: &dyn Host,
        entity: BlockEntityRef,
    ) -> Result<Option<Self::Data>, PartStateError> {
        read_state(host, entity, Self::DATA)
    }

    /// Changes the block entity's value with `edit`, starting from the default when it has
    /// none, and stores the result: see [`edit_part_state`].
    fn edit_state<R>(
        host: &mut dyn ServerHost,
        entity: BlockEntityRef,
        edit: impl FnOnce(&mut dyn ServerHost, &mut Self::Data) -> R,
    ) -> Result<R, PartStateError> {
        edit_part_state(host, entity, Self::DATA, edit)
    }
}

/// Why a block entity's or an entity's Part state could not be read or changed. Nothing was
/// stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: returned inside the mod that called; the calls it reports on cross as attachment calls.
pub enum PartStateError {
    /// The handle names a block entity or an entity of another level than the one this host
    /// is in.
    OtherLevel,
    /// The holder was removed, replaced or unloaded, or its world closed: get a fresh
    /// handle. A reloaded holder keeps its saved value under a new handle. An entity that
    /// changed level without a player riding it is rebuilt under a new id, and its state goes
    /// with it: find it again by its UUID. Inside an action that is rolled back after the
    /// entity changed level, the state stays with the entity under its new id, and the old
    /// handle stays `Stale`.
    Stale,
    /// The entity is out of its level's lists for its own tick, so its state can't be reached
    /// until that tick ends.
    HolderBusy,
    /// An edit of this holder's same state is already running further up the call:
    /// the inner one is refused rather than overwritten by the outer one.
    Reentrant,
    /// Something else changed the value while the edit's closure ran; the edit stored
    /// nothing and its transaction was aborted.
    Changed,
    /// The attachment store refused the value: it does not decode (its saved bytes are
    /// kept), does not encode, or the key holds another Rust type.
    Attachment {
        /// The state's namespace.
        namespace: &'static str,
        /// The state's name.
        name: &'static str,
        /// What the store said.
        error: AttachmentError,
    },
}

impl fmt::Display for PartStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OtherLevel => f.write_str("the holder is in another level than this host"),
            Self::Stale => f.write_str("the holder was removed, replaced, unloaded or moved"),
            Self::HolderBusy => f.write_str("the entity is out of its level for its own tick"),
            Self::Reentrant => f.write_str("this holder's state is already being edited"),
            Self::Changed => f.write_str("the state changed while it was being edited"),
            Self::Attachment {
                namespace,
                name,
                error,
            } => write!(f, "part state {namespace}:{name}: {error:?}"),
        }
    }
}

impl std::error::Error for PartStateError {}

/// Why a Part's state declaration was refused at registration.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PartStateRefusal {
    /// `DATA.sync` is set: Part state is not sent to clients yet.
    Sync,
    /// `DATA.copy_on_death` is set: a block entity has no death to copy across.
    CopyOnDeath,
    /// `DATA` has no codec, which a wasm mod needs: its values cross the wasm
    /// boundary only in their saved form.
    NoCodecOnWasm,
    /// `Data`'s Wire form holds a resource handle (an erased handler such as
    /// `Arc<dyn BlockPart>`), which crosses only into one link group's resource scope:
    /// not as data both targets read, nor into a save.
    ResourceHandle,
    /// `DATA`'s namespace is not one this mod owns.
    UnownedNamespace,
    /// The same key is already declared with a type of another Wire layout, named here.
    Incompatible {
        /// The type the key was first declared with.
        declared: &'static str,
    },
    /// The entity type keeps no entity Part state: a player, a vanilla family no entity part
    /// runs on, or no type at all. Decided once every type is known, when the game loads.
    EntityKind {
        /// The entity type id the Part was attached to.
        kind: &'static str,
        /// Why that type is refused.
        reason: &'static str,
    },
}

impl PartStateRefusal {
    /// What the refusal says, for a holder named `holder` (`block entities`, `entities`).
    pub(crate) fn describe(&self, f: &mut fmt::Formatter<'_>, holder: &str) -> fmt::Result {
        match self {
            Self::Sync => f.write_str("sync is not supported for part state yet"),
            Self::CopyOnDeath => write!(f, "copy_on_death does not apply to {holder}"),
            Self::NoCodecOnWasm => f.write_str(
                "a wasm mod's part state needs a codec (its values cross only in saved form)",
            ),
            Self::ResourceHandle => f.write_str(
                "its Wire form holds a resource handle, which crosses only within one link \
                 group: not to every target, nor into a save",
            ),
            Self::UnownedNamespace => f.write_str("the namespace is not one this mod owns"),
            Self::Incompatible { declared } => write!(
                f,
                "the key is already declared with type {declared}, whose Wire layout differs"
            ),
            Self::EntityKind { kind, reason } => write!(f, "entity type {kind}: {reason}"),
        }
    }
}

/// A Part with state could not be attached.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlockEntityStateError {
    /// The Part itself was refused, as by `block_entity_part`.
    Part(BlockEntityPartError),
    /// Its state declaration was refused.
    State {
        /// The Part type.
        part: &'static str,
        /// The state's namespace.
        namespace: &'static str,
        /// The state's name.
        name: &'static str,
        /// The state's Rust type.
        data: &'static str,
        /// Why.
        reason: PartStateRefusal,
    },
}

impl fmt::Display for BlockEntityStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::State {
            part,
            namespace,
            name,
            data,
            reason,
        } = self
        else {
            let Self::Part(error) = self else {
                unreachable!()
            };
            return error.fmt(f);
        };
        write!(
            f,
            "part {part}: state {namespace}:{name} of type {data} refused: "
        )?;
        reason.describe(f, "block entities")
    }
}

impl std::error::Error for BlockEntityStateError {}

impl From<BlockEntityPartError> for BlockEntityStateError {
    fn from(error: BlockEntityPartError) -> Self {
        Self::Part(error)
    }
}

/// One Part state declaration, as the registrar checks it.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PartStateDeclaration {
    /// The state's namespace.
    pub namespace: &'static str,
    /// The state's name.
    pub name: &'static str,
    /// The state type's Wire layout: its schema across the native and wasm boundary, which two
    /// declarations of one key must share.
    pub layout: u64,
    /// Its name, for a refusal.
    pub data_name: &'static str,
    sync: bool,
    copy_on_death: bool,
    codec: bool,
    /// Whether the type's Wire form holds a resource handle anywhere in it.
    resource: bool,
}

impl PartStateDeclaration {
    /// The declaration of `key`: what the typed declaration is erased to, the same on both
    /// targets.
    pub fn of<T: crate::wire::Wire + 'static>(key: &AttachmentType<T>) -> Self {
        let (layout, resource) = crate::wire::resource_layout::<T>();
        Self {
            namespace: key.namespace,
            name: key.name,
            layout,
            data_name: std::any::type_name::<T>(),
            sync: key.sync,
            copy_on_death: key.copy_on_death,
            codec: key.codec.is_some(),
            resource,
        }
    }

    /// What the declaration is refused for on its own, for a mod the game loaded as a wasm
    /// module where `wasm` (the game's knowledge, not the module's): syncing, copying on death,
    /// a resource handle in its Wire form, or no codec in a wasm mod.
    pub fn check(&self, wasm: bool) -> Result<(), PartStateRefusal> {
        if self.sync {
            return Err(PartStateRefusal::Sync);
        }
        if self.copy_on_death {
            return Err(PartStateRefusal::CopyOnDeath);
        }
        if self.resource {
            return Err(PartStateRefusal::ResourceHandle);
        }
        if wasm && !self.codec {
            return Err(PartStateRefusal::NoCodecOnWasm);
        }
        Ok(())
    }

    /// The error refusing it for `reason`, for entity Part `part` attached to `kind`.
    pub fn refused_on_entity(
        &self,
        part: &'static str,
        kind: &'static str,
        reason: PartStateRefusal,
    ) -> crate::EntityPartStateError {
        crate::EntityPartStateError::State {
            part,
            kind,
            namespace: self.namespace,
            name: self.name,
            data: self.data_name,
            reason,
        }
    }

    /// The error refusing it for `reason`, for Part `part`.
    pub fn refused(&self, part: &'static str, reason: PartStateRefusal) -> BlockEntityStateError {
        BlockEntityStateError::State {
            part,
            namespace: self.namespace,
            name: self.name,
            data: self.data_name,
            reason,
        }
    }
}

/// A holder Part state is kept on: a scoped handle that a call checks against the host before
/// it reaches the holder's attachment map. Sealed: the game decides which holders keep it.
pub trait PartStateHolder: Copy + PartialEq + private::Sealed + 'static {
    /// Checks the handle against this host's level and the holder it names now, and gives the
    /// level-relative holder its value is stored under.
    // hidden: the Part state core's own step; mods call `state` and `edit_state`
    #[doc(hidden)]
    fn part_state_holder(self, host: &dyn Host) -> Result<Holder, PartStateError>;
    /// The handle as the attachment index names it.
    // hidden: the Part state core's own step; mods call `state` and `edit_state`
    #[doc(hidden)]
    fn attachment_holder(self) -> AttachmentHolder;
}

mod private {
    pub trait Sealed {}
    impl Sealed for crate::BlockEntityRef {}
    impl Sealed for crate::EntityRef {}
}

impl PartStateHolder for BlockEntityRef {
    fn part_state_holder(self, host: &dyn Host) -> Result<Holder, PartStateError> {
        if host.level(host.dimension()) != Some(self.level) {
            return Err(PartStateError::OtherLevel);
        }
        if host.block_entity_identity(self.level, self.position) != Some(self) {
            return Err(PartStateError::Stale);
        }
        Ok(Holder::BlockEntity(self.position))
    }

    fn attachment_holder(self) -> AttachmentHolder {
        AttachmentHolder::BlockEntity(self)
    }
}

fn attachment<T>(key: &AttachmentType<T>, error: AttachmentError) -> PartStateError {
    match error {
        AttachmentError::MissingHolder => PartStateError::Stale,
        error => PartStateError::Attachment {
            namespace: key.namespace,
            name: key.name,
            error,
        },
    }
}

/// [`BlockEntityPartState::state_existing`] for any key.
pub fn read_state<H: PartStateHolder, T: Clone + Send + Sync + 'static>(
    host: &dyn Host,
    entity: H,
    key: &AttachmentType<T>,
) -> Result<Option<T>, PartStateError> {
    entity.part_state_holder(host)?;
    host.query(key)
        .map_err(|error| attachment(key, error))?
        .get(entity.attachment_holder())
        .map_err(|error| match error {
            AttachmentRowError::Value(error) => attachment(key, error),
            _ => PartStateError::Stale,
        })
}

thread_local! {
    /// The edits running on this thread, innermost last.
    static EDITING: RefCell<Vec<(AttachmentHolder, &'static str, &'static str)>> =
        const { RefCell::new(Vec::new()) };
}

/// One running edit, taken off [`EDITING`] however the edit ends.
struct Editing;

impl Editing {
    fn enter<T>(
        entity: AttachmentHolder,
        key: &'static AttachmentType<T>,
    ) -> Result<Self, PartStateError> {
        EDITING.with_borrow_mut(|editing| {
            let this = (entity, key.namespace, key.name);
            if editing.contains(&this) {
                return Err(PartStateError::Reentrant);
            }
            editing.push(this);
            Ok(Self)
        })
    }
}

impl Drop for Editing {
    fn drop(&mut self) {
        EDITING.with_borrow_mut(|editing| editing.pop());
    }
}

/// [`BlockEntityPartState::edit_state`] or [`crate::EntityPartState::edit_state`] for any key.
///
/// Runs in a transaction of its own, nested in the open one if any: the value is read
/// (decoded once from its saved form, or the default when absent), lent to `edit` as a
/// copy while no store borrow is held, and stored back after `edit` returns, if the holder
/// is still the one `entity` names and nothing else replaced the value meanwhile.
/// Otherwise, or when the value does not encode, the transaction aborts with everything
/// `edit` wrote and the old value stays, with its saved bytes. A panic in `edit` aborts it
/// too. An undecodable saved value refuses before `edit` runs and is kept as it was.
/// An edit of the same holder and key from inside `edit` is refused.
pub fn edit_part_state<H: PartStateHolder, T: Clone + Send + Sync + 'static, R>(
    host: &mut dyn ServerHost,
    entity: H,
    key: &'static AttachmentType<T>,
    edit: impl FnOnce(&mut dyn ServerHost, &mut T) -> R,
) -> Result<R, PartStateError> {
    let _editing = Editing::enter(entity.attachment_holder(), key)?;
    host.transaction(|host, _| {
        let at = entity.part_state_holder(host)?;
        let store = host.attachments();
        let current = key
            .get_existing(store, at)
            .map_err(|error| attachment(key, error))?;
        // The value's own allocation: each store makes a new one and a transaction puts
        // the same one back, so it changed exactly when this differs. Holding it keeps
        // its address from being reused.
        let revision = store
            .decoded(at, key.namespace, key.name)
            .map_err(|error| attachment(key, error))?;
        let mut value = current.unwrap_or_else(key.default);
        let answer = edit(host, &mut value);
        entity.part_state_holder(host)?;
        let store = host.attachments();
        let now = store
            .decoded(at, key.namespace, key.name)
            .map_err(|error| attachment(key, error))?;
        if !same(revision.as_ref(), now.as_ref()) {
            return Err(PartStateError::Changed);
        }
        key.set(store, at, value)
            .map_err(|error| attachment(key, error))?;
        Ok(answer)
    })
}

fn same(a: Option<&AttachmentValue>, b: Option<&AttachmentValue>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn key(sync: bool, copy_on_death: bool, codec: bool) -> AttachmentType<i32> {
        AttachmentType {
            namespace: "test",
            name: "state",
            default: || 0,
            codec: match codec {
                true => Some(AttachmentCodec {
                    encode: |_| Ok(Compound::new()),
                    decode: |_| Ok(0),
                }),
                false => None,
            },
            copy_on_death,
            sync,
        }
    }

    #[test]
    fn declarations_refuse_sync_copy_on_death_handles_and_codec_less_wasm_state() {
        let check = |key: &AttachmentType<i32>, wasm| PartStateDeclaration::of(key).check(wasm);
        assert_eq!(check(&key(false, false, true), true), Ok(()));
        assert_eq!(check(&key(false, false, false), false), Ok(()));
        assert_eq!(
            check(&key(false, false, false), true),
            Err(PartStateRefusal::NoCodecOnWasm)
        );
        assert_eq!(
            check(&key(true, false, true), false),
            Err(PartStateRefusal::Sync)
        );
        assert_eq!(
            check(&key(false, true, true), false),
            Err(PartStateRefusal::CopyOnDeath)
        );
        // A handle anywhere in the type, however deep, on either target.
        type Held = Option<Vec<(u8, Arc<dyn crate::BlockPart>)>>;
        static HELD: AttachmentType<Held> = AttachmentType {
            namespace: "test",
            name: "held",
            default: || None,
            codec: None,
            copy_on_death: false,
            sync: false,
        };
        for wasm in [false, true] {
            assert_eq!(
                PartStateDeclaration::of(&HELD).check(wasm),
                Err(PartStateRefusal::ResourceHandle)
            );
        }
    }

    #[test]
    fn a_refusal_names_the_part_key_type_and_capability() {
        let error = BlockEntityStateError::State {
            part: "press::Press",
            namespace: "press",
            name: "progress",
            data: "u32",
            reason: PartStateRefusal::NoCodecOnWasm,
        };
        assert_eq!(
            error.to_string(),
            "part press::Press: state press:progress of type u32 refused: \
             a wasm mod's part state needs a codec (its values cross only in saved form)"
        );
    }

    #[test]
    fn revisions_compare_allocations_not_values() {
        let a: AttachmentValue = Arc::new(7_i32);
        let b: AttachmentValue = Arc::new(7_i32);
        assert!(same(Some(&a), Some(&a.clone())));
        assert!(!same(Some(&a), Some(&b)));
        assert!(same(None, None));
        assert!(!same(Some(&a), None));
    }
}
