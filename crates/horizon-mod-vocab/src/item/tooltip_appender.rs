//! A data component's own tooltip lines (NeoForge's `RegisterTooltipAppendersEvent`): a mod
//! registers an appender for one component with `Registrar::register_tooltip_appender`, and the
//! game asks it for a stack that carries the component and whose tooltip shows it.

use crate::{ClientHost, ItemStack, Text};

/// Where a component's lines go among the other components' lines (NeoForge's
/// `registerComponentAppenderBeforeAll`, `Before`, `After` and `AfterAll`). `Before` and `After`
/// name a component detail the game lists (`minecraft:lore`, `minecraft:enchantments`...); any
/// other id refuses the mods at startup. Only the 23 vanilla details are anchors;
/// the `registerAppender(TooltipLocation, ...)` overload is not represented.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum TooltipPlacement {
    /// Before every component's lines, after the item's own lines.
    BeforeAll,
    /// Just before the lines of the component named.
    Before(String),
    /// Just after the lines of the component named.
    After(String),
    /// After every component's lines, before the durability and advanced lines.
    AfterAll,
}

/// What a tooltip is about, as its appender gets it: the local player and the tooltip's flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct TooltipAppendContext {
    /// The local player's UUID, including for nested projectiles where vanilla passes no player.
    pub player: u128,
    /// The local player's entity id.
    pub player_entity_id: i32,
    /// `TooltipFlag.isAdvanced` (F3+H).
    pub advanced: bool,
    /// `TooltipFlag.isCreative`: a creative tab's own slot.
    pub creative: bool,
    /// A crossbow's loaded projectile, whose details its crossbow lists.
    pub nested: bool,
}

/// The lines a data component adds to its stack's tooltip. Kept by the game from registration on;
/// asked only for a stack that carries the component and whose `tooltip_display` shows it, and
/// only while its mod is enabled. Multiple registrations for the same component all run.
pub trait TooltipAppender: Send + Sync + 'static {
    /// Add lines to `lines` for `stack`, each with a colour as `rgba` packs it (`u32::MAX` keeps
    /// the text's own). `context` is the tooltip's player and flags.
    fn append(
        &self,
        host: &dyn ClientHost,
        stack: &ItemStack,
        context: &TooltipAppendContext,
        lines: &mut Vec<(Text, u32)>,
    );
}
