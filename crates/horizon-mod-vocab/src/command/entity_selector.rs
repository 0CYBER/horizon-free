//! Entity selector tokens a mod registers (`Registrar::register_entity_selector`): `@allies`
//! beside vanilla's `@p @a @r @s @e @n`, in every entity and player argument (NeoForge's
//! `EntitySelectorManager` and `IEntitySelectorType`).

use crate::wire::{Reader, Wire, WireError, Writer};
use crate::{CommandSource, EntityRef, ServerHost};
use std::sync::Arc;

/// How a registered selector token orders its candidates before its limit, as `sort=` does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum SelectorSort {
    /// `sort=arbitrary`: the level's order, as `@e` and `@a`.
    #[default]
    Arbitrary,
    /// `sort=nearest`, as `@p` and `@n`.
    Nearest,
    /// `sort=furthest`.
    Furthest,
    /// `sort=random`, as `@r`.
    Random,
}

/// The base selection of a registered token, before its `[...]` options: what `@p` or `@e`
/// set for themselves. Options a player writes after the token apply on top, as for `@e`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct SelectorBase {
    /// Only players are candidates, as for `@a` (`type=minecraft:player`).
    pub players_only: bool,
    /// At most this many results; `None` for no limit. `Some(1)` lets a single-entity
    /// argument (`entity`, `player`) take the token. Zero refuses registration.
    pub limit: Option<u32>,
    /// How the candidates are ordered before the limit.
    pub sort: SelectorSort,
    /// The translation key of the completion's tooltip.
    pub tooltip: String,
}

impl SelectorBase {
    /// A base selection with its tooltip key.
    pub fn new(
        players_only: bool,
        limit: Option<u32>,
        sort: SelectorSort,
        tooltip: impl Into<String>,
    ) -> Self {
        Self {
            players_only,
            limit,
            sort,
            tooltip: tooltip.into(),
        }
    }
}

/// A selector token a mod registers: its base selection and which candidates it keeps.
///
/// Register it once, from `Hooks::register`, with `Registrar::register_entity_selector`. Its
/// name is the word after `@`. The server parses `@name[...]` in every entity, entities,
/// player and players argument, vanilla commands included; selector permission is checked as
/// for vanilla selectors. Horizon clients receive token metadata beside the command tree;
/// vanilla clients may still send the command without local token completion.
///
/// The game keeps one value per token and calls it on the server thread: implement it to be
/// shared, without mutable state.
pub trait EntitySelectorType: Send + Sync + 'static {
    /// The base selection, read once at registration and frozen for the process.
    /// A panic, zero limit, or a tooltip longer than 32767 UTF-16 units refuses registration.
    fn base(&self) -> SelectorBase;

    /// Whether `entity` stays selected, asked for each candidate after the base selection
    /// and the `[...]` options, before sorting and the limit. `source` is the command's origin.
    fn matches(&self, host: &mut dyn ServerHost, source: &CommandSource, entity: EntityRef)
    -> bool;
}

crate::wire::wire_handlers::handler!(dyn EntitySelectorType, "EntitySelectorType");
