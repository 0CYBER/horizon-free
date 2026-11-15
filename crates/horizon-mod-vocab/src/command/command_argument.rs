//! Command argument types a mod registers (`Registrar::register_command_argument`): parsing and
//! tab-completion for one kind of value in a `Command` usage.
//!
//! A client that does not know the type still parses the command: the wire tree sends the
//! argument as a Brigadier string (one word, or the rest of the line) and asks the server for its
//! completions. Fabric instead registers the type in the command-argument registry, whose id an
//! unmodded client cannot read and whose node it drops; the string keeps a vanilla tree valid.

use crate::{PermissionLevel, Text};

/// How much of the command line a custom argument's value takes.
///
/// The server and a client that does not know the type parse the value as this before the mod's
/// own parser sees it, so the type must say it up front, when it is registered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum CommandArgumentKind {
    /// One word, as Brigadier's `word` string: ASCII letters and digits, and `_ - . +`. The word
    /// ends at the first other character, a space included, so a value containing `:` or `/`
    /// needs [`Greedy`](Self::Greedy) as the usage's last argument.
    #[default]
    Word,
    /// The rest of the line, spaces included: the argument must be the usage's last.
    Greedy,
}

/// Why a custom argument's parser refused the text a player wrote.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum CommandArgumentError {
    /// The message shown to the player.
    Message(Box<Text>),
}

impl CommandArgumentError {
    /// A refusal with the message the player sees.
    pub fn new(message: Text) -> Self {
        Self::Message(Box::new(message))
    }

    /// The message the player sees.
    pub fn message(&self) -> &Text {
        let CommandArgumentError::Message(message) = self;
        message
    }
}

impl From<(crate::Identifier, String)> for CommandArgumentError {
    fn from((_, message): (crate::Identifier, String)) -> Self {
        Self::new(Text::literal(message))
    }
}

impl From<CommandArgumentError> for Text {
    fn from(error: CommandArgumentError) -> Self {
        let CommandArgumentError::Message(message) = error;
        *message
    }
}

/// One completion a custom argument offers for the text being typed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct CommandSuggestion {
    /// The text that replaces the typed prefix, or the part of it
    /// [`replace_start`](Self::replace_start) sends the replacement from.
    pub text: String,
    /// An optional explanation shown beside the completion.
    pub tooltip: Option<String>,
    /// The byte offset into the prefix where the replacement starts; `None` replaces the whole
    /// prefix, as a completion does by default.
    ///
    /// A greedy text argument holds several tokens, and a player typing the last of them wants
    /// one completion for it, not a rewrite of the rest: for the prefix `stone st` and an offset
    /// of 6, the suggestion `stone` replaces `st` and keeps `stone ` in front of it. The offset
    /// must be inside the prefix and on a character boundary; a suggestion this build cannot
    /// apply is refused (reported, and left out) rather than panicking on the player's input.
    pub replace_start: Option<usize>,
}

impl CommandSuggestion {
    /// A completion without a tooltip that replaces the whole typed prefix.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tooltip: None,
            replace_start: None,
        }
    }

    /// A completion with an explanation.
    pub fn with_tooltip(text: impl Into<String>, tooltip: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tooltip: Some(tooltip.into()),
            replace_start: None,
        }
    }

    /// The same completion, replacing the prefix from byte `replace_start` on.
    pub fn replacing_from(mut self, replace_start: usize) -> Self {
        self.replace_start = Some(replace_start);
        self
    }
}

/// The player a completion is for: the one typing, with the level they run commands at.
///
/// [`CommandArgumentType::suggest`] receives it, so a provider that completes from a mod's own
/// registry answers with what that sender may use and nothing else. The game keeps the answer as
/// it comes: it does not filter a provider's completions, and a provider that ignores the sender
/// offers everything to everyone.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
// exhaustive: the game builds it (`mod_api::__private::game::command_sender`), mods only read it
pub struct CommandSender {
    pub(crate) uuid: u128,
    pub(crate) name: String,
    pub(crate) permission: PermissionLevel,
}

impl CommandSender {
    /// The player's UUID.
    pub fn uuid(&self) -> u128 {
        self.uuid
    }

    /// The player's account name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The level they run commands at, the same one a usage's `level` is compared with.
    pub fn permission(&self) -> PermissionLevel {
        self.permission
    }
}

impl Default for CommandSender {
    fn default() -> Self {
        Self {
            uuid: 0,
            name: String::new(),
            permission: PermissionLevel::All,
        }
    }
}

/// A command argument type a mod registers: how one kind of argument is validated and completed.
///
/// Register it once, from `Hooks::register`, with `Registrar::register_command_argument`. Its
/// namespaced id (`mymod:machine`) is the kind a `Command` usage names after the argument
/// name: `Command::new("machine use <machine:mymod:machine>")`. The game then parses the
/// argument with [`parse`](Self::parse) before calling `run_mod_command`, and asks
/// [`suggest`](Self::suggest) for the chat box's completions. Players' clients that do not know
/// the type complete it from the server's answer.
///
/// The game keeps one value per registered id and asks it from the server thread and, for a
/// client command, from the window's thread: implement it to be shared, without mutable state.
pub trait CommandArgumentType: Send + Sync + 'static {
    /// The part of the line the argument takes. The same on every client.
    fn kind(&self) -> CommandArgumentKind;

    /// Validate the text a client wrote and return the value `run_mod_command` receives.
    ///
    /// `input` is one word (see [`CommandArgumentKind::Word`] for its characters), or the rest
    /// of the line for [`CommandArgumentKind::Greedy`]. Return the canonical text to deliver (an
    /// id, a lower-cased name, `"3"` for `"03"`), or the message shown to the player when the
    /// input is not valid.
    fn parse(&self, input: &str) -> Result<String, CommandArgumentError>;

    /// The completions for the text `prefix` being typed at this argument, for `sender`.
    ///
    /// `prefix` is the text the argument holds so far: one word's worth for
    /// [`Word`](CommandArgumentKind::Word), and the whole tail of the line up to the cursor for
    /// [`Greedy`](CommandArgumentKind::Greedy). Return the values that match, in the order the
    /// chat should show them; the game keeps each suggestion's replacement range and tooltip. A
    /// suggestion replaces the whole prefix unless it names a
    /// [`replace_start`](CommandSuggestion::replace_start), which is how a type completes the
    /// token being typed inside a longer greedy text. An empty list leaves the input as typed.
    ///
    /// `sender` is the player asking, with their permission level: a provider that completes names
    /// from a mod's registry returns only the entries that sender may use, because the game keeps
    /// its answer unfiltered. It is `None` when no player asks: the console, and a client
    /// completing its own client commands before its own profile is known.
    ///
    /// Native and wasm results share the vanilla packet limits: completion text may contain
    /// at most 32767 UTF-16 code units; a literal tooltip may occupy at most 65535 bytes in
    /// Java modified UTF-8 (the NBT string limit). If any result exceeds either limit, the
    /// engine reports the refusal and discards the callback's results, rather than truncating
    /// player-visible text. A panicking native callback is also refused.
    fn suggest(&self, sender: Option<&CommandSender>, prefix: &str) -> Vec<CommandSuggestion>;
}

/// A ready argument type that takes one of a fixed set of words: NeoForge's `EnumArgument`.
///
/// Register it like any type: `r.register_command_argument("mode", ChoiceArgument::new(["low",
/// "high"]))`, then name `<mode:mymod:mode>` in a usage. A word outside the set is refused with
/// the valid ones listed; completion offers the choices that start with the typed text, in the
/// order given. The value arrives as `CommandArgumentValue::Custom` with the chosen word.
// no wire: a registered argument type stays in its mod; only its parse and completion results cross.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ChoiceArgument {
    choices: Vec<String>,
}

impl ChoiceArgument {
    /// The words a player may write, in completion order. Each must be a Brigadier `word`.
    pub fn new(choices: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            choices: choices.into_iter().map(Into::into).collect(),
        }
    }
}

impl CommandArgumentType for ChoiceArgument {
    fn kind(&self) -> CommandArgumentKind {
        CommandArgumentKind::Word
    }

    fn parse(&self, input: &str) -> Result<String, CommandArgumentError> {
        if self.choices.iter().any(|choice| choice == input) {
            return Ok(input.to_owned());
        }
        // NeoForge's key; a client without it shows the fallback.
        Err(CommandArgumentError::new(Text::new(
            crate::text::TextContent::Translatable {
                key: "commands.neoforge.arguments.enum.invalid".into(),
                fallback: Some("Enum constant must be one of %s, found %s".into()),
                arguments: vec![Text::literal(self.choices.join(", ")), Text::literal(input)],
            },
        )))
    }

    fn suggest(&self, _: Option<&CommandSender>, prefix: &str) -> Vec<CommandSuggestion> {
        self.choices
            .iter()
            .filter(|choice| choice.starts_with(prefix))
            .map(CommandSuggestion::new)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_choice_refuses_other_words_and_completes_by_prefix() {
        let mode = ChoiceArgument::new(["low", "high"]);
        assert_eq!(mode.parse("high"), Ok("high".into()));
        let refusal = mode.parse("med").unwrap_err();
        let crate::text::TextContent::Translatable { arguments, .. } = &refusal.message().content
        else {
            panic!("a translatable refusal");
        };
        assert_eq!(
            arguments,
            &vec![Text::literal("low, high"), Text::literal("med")]
        );
        assert_eq!(
            mode.suggest(None, "h"),
            vec![CommandSuggestion::new("high")]
        );
    }

    #[test]
    fn an_id_value_crosses_the_wire() {
        use super::super::CommandArgumentValue;
        let value = CommandArgumentValue::Id(crate::Identifier::parse("minecraft:zombie").unwrap());
        let mut out = crate::wire::Writer::new();
        out.put(&value);
        assert_eq!(
            crate::wire::Reader::decode::<CommandArgumentValue>(out.bytes()).unwrap(),
            value
        );
    }

    #[test]
    fn a_suggestion_crosses_the_wire_with_its_replacement_start() {
        for suggestion in [
            CommandSuggestion::new("stone"),
            CommandSuggestion::with_tooltip("stone", "a block").replacing_from(6),
        ] {
            let mut out = crate::wire::Writer::new();
            out.put(&suggestion);
            assert_eq!(
                crate::wire::Reader::decode::<CommandSuggestion>(out.bytes()).unwrap(),
                suggestion
            );
        }
    }
}
