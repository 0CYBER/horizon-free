//! The item model properties, tint sources and node types native mods register (NeoForge's
//! `RegisterRangeSelectItemModelPropertyEvent`, `RegisterSelectItemModelPropertyEvent`,
//! `RegisterColorHandlersEvent.ItemTintSources` and `RegisterItemModelsEvent`). A
//! `range_dispatch` or `select` on a registered property, a tint of a registered source and a node
//! of a registered type read their answer from the host per stack; a property or tint nobody
//! registered stays foreign (`foreign.rs`), and a node type nobody registered fails to decode.
//! The special model types native mods register (`RegisterSpecialModelRendererEvent`) decode
//! in a `minecraft:special` model, which the host draws per stack, vanilla's types included; one
//! nobody registered that vanilla doesn't have fails to decode.
//! The skull types native mods register (`EntityRenderersEvent.CreateSkullModels`) are kinds a
//! `minecraft:head` model may name.
use super::identifier;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::RwLock,
};

static RANGE: RwLock<BTreeSet<String>> = RwLock::new(BTreeSet::new());
static SELECT: RwLock<BTreeSet<String>> = RwLock::new(BTreeSet::new());
static TINT: RwLock<BTreeSet<String>> = RwLock::new(BTreeSet::new());
static NODE: RwLock<BTreeSet<String>> = RwLock::new(BTreeSet::new());
static SKULL: RwLock<BTreeSet<String>> = RwLock::new(BTreeSet::new());
static SPECIAL: RwLock<BTreeMap<String, SpecialModelSlot>> = RwLock::new(BTreeMap::new());

/// A registered special model type, by its place among the ids
/// [`register_special_model_types`] kept: resolved when a model naming it decodes, so drawing it
/// looks nothing up by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpecialModelSlot(pub u32);

/// Keeps the numeric `range_dispatch` properties native mods registered, before
/// resources load. An id that is not `<namespace>:<path>` in a namespace other than
/// vanilla's is logged and left out. Returns the ids kept, in registration order.
pub fn register_range_properties(ids: Vec<String>) -> Vec<String> {
    keep_mod_ids(&RANGE, "range select item model property", ids)
}

/// Keeps the categorical `select` properties native mods registered, before resources
/// load, checked as [`register_range_properties`] checks its ids.
pub fn register_select_properties(ids: Vec<String>) -> Vec<String> {
    keep_mod_ids(&SELECT, "select item model property", ids)
}

/// Keeps the item tint sources native mods registered, before resources load, checked as
/// [`register_range_properties`] checks its ids.
pub fn register_tint_sources(ids: Vec<String>) -> Vec<String> {
    keep_mod_ids(&TINT, "item tint source", ids)
}

/// Keeps the skull types native mods registered, before resources load, checked as
/// [`register_range_properties`] checks its ids.
pub fn register_skull_kinds(ids: Vec<String>) -> Vec<String> {
    keep_mod_ids(&SKULL, "skull type", ids)
}

/// Adds to `table` the ids a mod registered as `kind`: an id that is not `<namespace>:<path>` in a
/// namespace other than vanilla's, or one registered already, is logged and left out. Returns the
/// ids kept, in registration order.
pub fn keep_mod_ids(table: &RwLock<BTreeSet<String>>, kind: &str, ids: Vec<String>) -> Vec<String> {
    let mut kept = Vec::new();
    let mut set = table.write().unwrap_or_else(|e| e.into_inner());
    for id in ids {
        if !is_mod_id(&id) {
            log::warn!("{kind} {id:?} is not <namespace>:<path> in a mod's namespace");
        } else if !set.insert(id.clone()) {
            log::warn!("{kind} {id} is registered twice");
        } else {
            kept.push(id);
        }
    }
    kept
}

/// Whether `id` is `<namespace>:<path>`, written in full, in a namespace other than vanilla's: an
/// id a mod may register.
pub fn is_mod_id(id: &str) -> bool {
    identifier(id).is_ok_and(|parsed| parsed == id) && !id.starts_with("minecraft:")
}

/// Whether a native mod registered `id` as a numeric `range_dispatch` property.
pub fn is_range_property(id: &str) -> bool {
    RANGE.read().unwrap_or_else(|e| e.into_inner()).contains(id)
}

/// Whether a native mod registered `id` as a categorical `select` property.
pub fn is_select_property(id: &str) -> bool {
    SELECT
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .contains(id)
}

/// Keeps the item model node types native mods registered, before resources load, checked as
/// [`register_range_properties`] checks its ids.
pub fn register_item_models(ids: Vec<String>) -> Vec<String> {
    keep_mod_ids(&NODE, "item model type", ids)
}

/// Whether a native mod registered `id` as an item model node type.
pub fn is_item_model_type(id: &str) -> bool {
    NODE.read().unwrap_or_else(|e| e.into_inner()).contains(id)
}

/// Keeps the special model types native mods registered (`RegisterSpecialModelRendererEvent`),
/// before resources load, in place of any kept before: vanilla's types may be among them, as a
/// later registration replaces an earlier one. An id that is not `<namespace>:<path>`, written in
/// full, is logged and left out. Returns the ids kept: the slot of each is its index.
pub fn register_special_model_types(ids: Vec<String>) -> Vec<String> {
    let mut slots = SPECIAL.write().unwrap_or_else(|e| e.into_inner());
    slots.clear();
    let mut kept = Vec::new();
    for id in ids {
        if identifier(&id).is_ok_and(|parsed| parsed == id) {
            slots.insert(id.clone(), SpecialModelSlot(kept.len() as u32));
            kept.push(id);
        } else {
            log::warn!("special model type {id:?} is not <namespace>:<path>");
        }
    }
    kept
}

/// The slot of the special model type `id`, when a native mod registered it.
pub fn special_model_slot(id: &str) -> Option<SpecialModelSlot> {
    SPECIAL
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .copied()
}

/// Whether a native mod registered `id` as an item tint source.
pub fn is_tint_source(id: &str) -> bool {
    TINT.read().unwrap_or_else(|e| e.into_inner()).contains(id)
}

/// Whether a native mod registered `kind` as a skull type a `minecraft:head` model may name.
pub fn is_skull_kind(kind: &str) -> bool {
    SKULL
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .contains(kind)
}
