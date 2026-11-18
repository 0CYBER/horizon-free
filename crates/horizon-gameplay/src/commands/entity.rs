//! The entities a command names, as the game hands them out: an opaque [`EntityRef`] and what
//! the commands over entities need from the game.
use super::host::{CommandHost, R};
use horizon_client::{command::Context, scoreboard_cmd::Holder, text::Text};

/// One entity a selector found. The game decides what the two numbers mean; a command only
/// passes them back to the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntityRef {
    /// Which kind of entity (a player, a guest, a mob...), as the game numbers them.
    pub kind: u8,
    /// The entity's id within its kind.
    pub id: u32,
}

/// What a command over entities needs from the game.
pub trait EntityHost: CommandHost {
    /// The entities the argument `name` selects (the implied player when it is absent);
    /// `players` allows player selectors.
    fn select_entities(&mut self, ctx: &Context, name: &str, players: bool) -> R<Vec<EntityRef>>;
    /// The entity's display name, as feedback shows it.
    fn entity_name(&self, entity: EntityRef) -> Text;
    /// The entity's position, if it has one.
    fn entity_position(&self, entity: EntityRef) -> Option<[f64; 3]>;
    /// A position argument of the command, resolved against its source.
    fn position_arg(&self, ctx: &Context, name: &str) -> Option<[f64; 3]>;
    /// The command source's position.
    fn source_position(&self) -> [f64; 3];
}

/// A number as feedback text.
pub fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}

/// A feedback holder with a name text only.
pub fn holder(text: Text) -> Holder {
    Holder {
        name: String::new(),
        text,
        display: None,
    }
}
