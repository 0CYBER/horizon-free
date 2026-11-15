//! Typed state an entity Part keeps on each entity of its type (arismoko/horizon#796 stage 3).
//!
//! The value lives in the entity's own attachment map, the store every other entity
//! attachment uses, through the same core as block entity Part state
//! ([`read_state`], [`edit_part_state`]): it saves with the entity through the type's
//! codec, is kept by transactions, and goes with the entity when it is removed, unloaded
//! or moved to another level.

use crate::*;
use std::fmt;

/// An entity Part's typed state: one value per entity of the Part's type, owned by the
/// game, not by the Part or a mod's statics.
///
/// Implement it beside [`EntityPart`] and attach the Part with
/// [`Registrar::entity_part_with_state`](crate::Registrar). Only mob types (vanilla and
/// mod) and mod general entity types keep it; a player, any other vanilla family and an
/// unknown type are refused when the game loads.
///
/// ```rust,ignore
/// struct Robot;
/// impl EntityPartState for Robot {
///     type Data = u32;
///     const DATA: &'static AttachmentType<u32> = &CHARGE;
/// }
/// // In a hook with `host: &mut dyn ServerHost` and the robot's `entity`:
/// let full = Robot::edit_state(host, entity, |_, charge| {
///     *charge += 1;
///     *charge >= 100
/// })?;
/// ```
///
/// Reads take `&dyn Host` and return a snapshot; edits take `&mut dyn ServerHost`, run in
/// their own nested transaction and store the new value only when the closure returns, as
/// [`BlockEntityPartState`]'s do. Every call first checks that `entity` is of the open
/// world and still names a resident entity, in the level this host is in:
/// - a removed, unloaded or moved entity refuses with [`PartStateError::Stale`];
/// - an entity in another level refuses with [`PartStateError::OtherLevel`];
/// - an entity out of its level's lists for its own tick refuses with
///   [`PartStateError::HolderBusy`].
///
/// An entity that changes level without a player riding it is rebuilt under a new id: the
/// old handle is `Stale` and the state arrives with the entity under its new id, so a mod
/// that needs continuity follows the entity's UUID. A vehicle a player rides keeps its id,
/// and its handle still resolves in the new level. Inside an action that is rolled back
/// after the entity changed level, the state stays with the entity under its new id, and
/// the old handle stays `Stale`. Travel fires no join or leave hook for it.
///
/// Server-side only for now: `DATA` must not sync or copy on death, and a client reads
/// the default.
pub trait EntityPartState: EntityPart + Sized {
    /// The value each entity of the Part's type keeps.
    type Data: Clone + Send + Sync + crate::wire::Wire + 'static;
    /// Its attachment key, default and codec. The key is in a namespace the mod owns;
    /// without a codec the value is kept for the session only.
    const DATA: &'static AttachmentType<Self::Data>;

    /// The entity's value, or the default when it has none. Inserts nothing.
    fn state(host: &dyn Host, entity: EntityRef) -> Result<Self::Data, PartStateError> {
        Ok(Self::state_existing(host, entity)?.unwrap_or_else(Self::DATA.default))
    }

    /// The entity's value, or `None` when it has none. Inserts nothing.
    fn state_existing(
        host: &dyn Host,
        entity: EntityRef,
    ) -> Result<Option<Self::Data>, PartStateError> {
        read_state(host, entity, Self::DATA)
    }

    /// Changes the entity's value with `edit`, starting from the default when it has none,
    /// and stores the result: see [`edit_part_state`]. A stale handle never creates a map.
    fn edit_state<R>(
        host: &mut dyn ServerHost,
        entity: EntityRef,
        edit: impl FnOnce(&mut dyn ServerHost, &mut Self::Data) -> R,
    ) -> Result<R, PartStateError> {
        edit_part_state(host, entity, Self::DATA, edit)
    }
}

/// Where the entity an [`EntityRef`] names is now, as its Part state checks it
/// ([`EntitiesHost::entity_residency`]).
#[derive(horizon_mod_api_macros::Wire, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum EntityResidency {
    /// Resident in the level this host is in, and reachable.
    InPlace,
    /// Resident in this host's level, but out of its lists for its own tick.
    Busy,
    /// Resident in another level of the open world.
    OtherLevel,
    /// No entity: removed (or its removal pending), unloaded, moved under a new id, or the
    /// handle is of another world open.
    #[default]
    Gone,
}

impl PartStateHolder for EntityRef {
    fn part_state_holder(self, host: &dyn Host) -> Result<Holder, PartStateError> {
        match host.entity_residency(self) {
            EntityResidency::InPlace => Ok(Holder::Entity(self.id())),
            EntityResidency::Busy => Err(PartStateError::HolderBusy),
            EntityResidency::OtherLevel => Err(PartStateError::OtherLevel),
            EntityResidency::Gone => Err(PartStateError::Stale),
        }
    }

    fn attachment_holder(self) -> AttachmentHolder {
        AttachmentHolder::Entity(self)
    }
}

/// An entity Part with state could not be attached: its state declaration was refused.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EntityPartStateError {
    /// Its state declaration was refused.
    State {
        /// The Part type.
        part: &'static str,
        /// The entity type it was attached to.
        kind: &'static str,
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

impl fmt::Display for EntityPartStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::State {
            part,
            kind,
            namespace,
            name,
            data,
            reason,
        } = self;
        write!(
            f,
            "entity part {part} on {kind}: state {namespace}:{name} of type {data} refused: "
        )?;
        reason.describe(
            f,
            "entities (only a player respawns, and players keep no entity part state)",
        )
    }
}

impl std::error::Error for EntityPartStateError {}

/// One entity Part state declaration as registration kept it, for the game to check its
/// entity type once every type is known.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct EntityPartStateDeclaration {
    /// The mod that declared it.
    pub mod_id: String,
    /// The Part type.
    pub part: &'static str,
    /// The entity type id the Part was attached to.
    pub kind: &'static str,
    /// The state declaration.
    pub state: PartStateDeclaration,
}

impl EntityPartStateDeclaration {
    /// The declaration registration keeps for mod `mod_id`'s Part `part` on type `kind`.
    pub fn new(
        mod_id: String,
        part: &'static str,
        kind: &'static str,
        state: PartStateDeclaration,
    ) -> Self {
        Self {
            mod_id,
            part,
            kind,
            state,
        }
    }

    /// The error refusing it for `reason`.
    pub fn refused(&self, reason: PartStateRefusal) -> EntityPartStateError {
        self.state.refused_on_entity(self.part, self.kind, reason)
    }

    /// The refusal for its entity type, if it has one: `minecraft:player`, a type that is
    /// neither a mob type (`mob`) nor a mod general entity type (`general`), named as another
    /// vanilla family when `known` holds the id and as an unknown type otherwise.
    pub fn kind_refusal(
        &self,
        mob: impl Fn(&str) -> bool,
        general: impl Fn(&str) -> bool,
        known: impl Fn(&str) -> bool,
    ) -> Option<PartStateRefusal> {
        let kind = self.kind;
        let reason = if kind == "minecraft:player" {
            "players: respawn identity is not decided"
        } else if mob(kind) || general(kind) {
            return None;
        } else if known(kind) {
            "no entity part runs on this kind"
        } else {
            "no such entity type"
        };
        Some(PartStateRefusal::EntityKind { kind, reason })
    }
}

/// The entity Part state declarations of every mod, in registration order.
pub fn entity_part_state_declarations() -> &'static [EntityPartStateDeclaration] {
    crate::parts::installed().map_or(&[], |installed| &installed.entity_part_states)
}
