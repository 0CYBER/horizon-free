//! Parts: code a mod attaches to one block, block entity, item or mod
//! menu type at startup. The scoped hooks ask the parts attached to the thing they are about,
//! after every mod's `Hooks` method. Parts attached by tag are resolved by each game against its
//! own tags, at world open and on `/reload`, into a table kept by that game's thread; that table
//! also indexes the block parts by state, so the state-keyed hooks look theirs up without the
//! block's name.
use crate::BlockEntityTypeDecl;
use crate::world::ticket_controller::ControllerInfo;
use crate::{
    BlockEntityPart, BlockPart, BlockSide, Capability, CapabilityKey, EntityCapabilityKey,
    EntityPart, EntityRef, FluidPart, HOOK_COUNT, HookId, Host, ItemCapabilityKey, ItemPart,
    ItemPlace, MenuPart, ModEntry, PlaceProvider, ServerHost, TicketController,
};
use std::{
    any::{Any, type_name},
    cell::RefCell,
    collections::{HashMap, HashSet},
    marker::PhantomData,
    rc::Rc,
    sync::{Arc, LazyLock, OnceLock, atomic::Ordering},
};

include!(concat!(env!("OUT_DIR"), "/parts_modules.rs"));

impl CapabilityProviders for InstalledProviders {}

#[cfg(test)]
use std::sync::atomic::AtomicBool;
