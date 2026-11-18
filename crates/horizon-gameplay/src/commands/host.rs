//! What every command needs from the game: telling the command source what happened.
//! A command family adds its own trait beside its logic, with this one as its supertrait.
use horizon_client::{
    command::{Arg, CommandError, Context},
    text::Text,
};

/// A command's result: the vanilla result value, or the error its source is shown.
pub type R<T> = Result<T, CommandError>;

/// An integer argument of the parsed command, if it has one.
pub fn int(ctx: &Context, name: &str) -> Option<i32> {
    match ctx.get(name) {
        Some(Arg::Int(v)) => Some(*v),
        _ => None,
    }
}

/// The feedback calls of a command source. Commands take `&mut dyn` of a trait with this
/// one as a supertrait, so a command's code is compiled here, in horizon-gameplay, and not
/// instantiated in horizon-client: an edit to it still re-checks horizon-client, but
/// incrementally, without generating the command's code there.
pub trait CommandHost {
    /// A vanilla `CommandSyntaxException` with a translated message: the game's own `err`, so a
    /// mod listening on it sees these errors too.
    fn error(&self, key: &str, args: &[&dyn std::fmt::Display]) -> CommandError;
    /// `sendSuccess(message, true)`: the source is told, and the operators when the rules say so.
    fn feedback(&mut self, message: Text);
    /// `sendSuccess(message, false)`: the source is told, the operators are not.
    fn feedback_unbroadcast(&mut self, message: Text);
}
