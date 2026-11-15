//! Owned authoring and validation; lookup storage stays below the engine.
mod_vocab::__mod_parts_authoring_imports!();
use mod_vocab::__private::game::RegistrationContextNew;
use mod_vocab::__private::*;
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    sync::{Arc, OnceLock, atomic::Ordering},
};
include!(concat!(env!("OUT_DIR"), "/parts_registration_modules.rs"));
impl Registrar for PartRegistrar {}

pub use shared::install_parts;

#[cfg(test)]
use mod_vocab::__private::game::PartsLookup;
#[cfg(test)]
use std::{any::type_name, collections::HashSet};
