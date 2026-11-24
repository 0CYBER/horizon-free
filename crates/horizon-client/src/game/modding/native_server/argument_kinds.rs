//! The argument kinds a native command usage names: the built-in ones, each a Brigadier
//! parser, and the types mods registered, with the dispatcher type each node parses as.

use super::{ArgType, Text, TryIntoEngine};
use horizon_client::command::{
    CommandError, Dispatcher, NodeKind, Range, Reader, args::Coordinates, snbt,
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, OnceLock},
};

/// The argument kinds a native usage may name, each with a [`parser`]; `resource:<registry>`
/// and `mod_id` come on top.
pub(super) const KINDS: [&str; 19] = [
    "word",
    "text",
    "integer",
    "number",
    "boolean",
    "player",
    "player_name",
    "entity",
    "entities",
    "players",
    "item",
    "block_pos",
    "vec3",
    "vec3_uncentered",
    "column_pos",
    "rotation",
    "dimension",
    "nbt",
    "id",
];

/// The tree property marking `vec3_uncentered`: vanilla's `vec3(false)` has the same parser id
/// and properties as `vec3()`, so the report format cannot tell them apart.
pub(super) const UNCENTERED: &str = "uncentered";

/// The tree key marking a custom argument of `CommandArgumentKind::Token`: the wire parser is a
/// Brigadier word, so the server's own dispatcher type is told apart by this key.
pub(super) const TOKEN: &str = "token";

/// The custom argument types the mods registered, by namespaced id, for the command tree being
/// built and its server dispatcher.
pub(in super::super) type CommandArguments = HashMap<String, Arc<dyn mod_api::CommandArgumentType>>;

/// The argument types the installed mods registered at startup, and the built-in `mod_id`:
/// NeoForge's `ModIdArgument`, one of the loaded package ids, delivered as a string.
pub(in super::super) fn command_arguments() -> CommandArguments {
    let mod_ids = mod_api::__private::loaded_mods()
        .ids()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mod_id: Arc<dyn mod_api::CommandArgumentType> =
        Arc::new(mod_api::ChoiceArgument::new(mod_ids));
    mod_api::__private::command_argument_types()
        .into_iter()
        .chain([(MOD_ID.to_owned(), mod_id)])
        .collect()
}

/// The built-in kind whose value is a loaded package id. Mods' kinds are namespaced, so it
/// never meets one.
pub(super) const MOD_ID: &str = "mod_id";

/// The most distinct registries native usages may name. Vanilla has about a hundred, so this
/// leaves mods room while keeping the names kept for the game's life bounded.
const MAX_REGISTRIES: usize = 256;
/// The longest registry name, in bytes: what the network side keeps of a received tree's names.
const MAX_REGISTRY_NAME: usize = 256;

/// The brigadier parser and properties of `resource:<registry>`: `ResourceKeyArgument` on the
/// wire, so vanilla clients complete it, and `ResourceArgument` on the server, which refuses an id
/// the registry lacks before the mod runs. A name that is not a resource location, or a registry
/// past the budget, refuses the usage with the reason.
pub(super) fn resource_parser(name: &str) -> Result<(&'static str, Value), String> {
    let registry = registry(name)?;
    Ok(("minecraft:resource_key", json!({ "registry": registry })))
}

/// A registry name a usage named, kept for the game's life (`ArgType::Res` holds
/// `&'static str`), each name once. Its own set, apart from the interner the network fills from
/// received trees, so neither spends the other's budget.
fn registry(name: &str) -> Result<&'static str, String> {
    static NAMES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut names = NAMES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    registry_in(&mut names, name)
}

fn registry_in(names: &mut HashSet<&'static str>, name: &str) -> Result<&'static str, String> {
    if let Some(&known) = names.get(name) {
        return Ok(known);
    }
    if name.len() > MAX_REGISTRY_NAME || mod_api::Identifier::parse_namespaced(name).is_none() {
        return Err(format!(
            "registry `{name}` is not a `namespace:path` resource location of at most \
             {MAX_REGISTRY_NAME} bytes"
        ));
    }
    if names.len() >= MAX_REGISTRIES {
        return Err(format!(
            "registry `{name}` is past the limit of {MAX_REGISTRIES} distinct registries"
        ));
    }
    let name: &'static str = Box::leak(name.into());
    names.insert(name);
    Ok(name)
}

/// What a mod receives for a custom argument: `mod_id` as the plain package id, a mod's own type
/// with its id.
pub(super) fn custom_value(kind: &str, value: String) -> mod_api::CommandArgumentValue {
    if kind == MOD_ID {
        mod_api::CommandArgumentValue::String(value)
    } else {
        mod_api::CommandArgumentValue::Custom {
            kind: kind.to_owned(),
            value,
        }
    }
}

/// Whether `word` is a usage's literal word: ASCII letters of either case, digits, `_` and `-`.
/// Brigadier's literals take any case (`/cu callItemClear`), matched as written. The first word
/// may also start with one `/`, so a mod can register WorldEdit-style `//set` (typed with the chat
/// slash): the node keeps the slash, and the client completes `//set ` against it.
pub(super) fn command_word(word: &str, first: bool) -> bool {
    let word = match first {
        true => word.strip_prefix('/').unwrap_or(word),
        false => word,
    };
    !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-".contains(c))
}

/// The brigadier parser of a built-in argument kind, its properties and its dispatcher type.
/// `EntityArgument.entity` / `entities` / `player` / `players` share the vanilla parser.
pub(super) fn parser(kind: &str) -> Option<(&'static str, Value, ArgType)> {
    Some(match kind {
        "word" => ("brigadier:string", json!({"type": "word"}), ArgType::Word),
        "text" => (
            "brigadier:string",
            json!({"type": "greedy"}),
            ArgType::Greedy,
        ),
        "integer" => (
            "brigadier:integer",
            Value::Null,
            ArgType::Integer {
                min: i32::MIN,
                max: i32::MAX,
            },
        ),
        "number" => (
            "brigadier:double",
            Value::Null,
            ArgType::Double {
                min: f64::MIN,
                max: f64::MAX,
            },
        ),
        "boolean" => ("brigadier:bool", Value::Null, ArgType::Bool),
        "entity" | "entities" | "player" | "players" => {
            let single = matches!(kind, "entity" | "player");
            let players = matches!(kind, "player" | "players");
            (
                "minecraft:entity",
                json!({
                    "amount": if single { "single" } else { "multiple" },
                    "type": if players { "players" } else { "entities" },
                }),
                ArgType::Entity { single, players },
            )
        }
        // `GameProfileArgument`: a player's name, online or not, completed from the online
        // players. The mod gets the typed name as a string, never a selector.
        "player_name" => ("minecraft:game_profile", Value::Null, ArgType::GameProfile),
        "item" => ("minecraft:item_stack", Value::Null, ArgType::Item),
        "block_pos" => ("minecraft:block_pos", Value::Null, ArgType::BlockPos),
        // `Vec3Argument.vec3()` centres whole x and z on their block, `vec3(false)` does not.
        "vec3" => (
            "minecraft:vec3",
            Value::Null,
            ArgType::Vec3 { center: true },
        ),
        "vec3_uncentered" => (
            "minecraft:vec3",
            Value::Null,
            ArgType::Vec3 { center: false },
        ),
        "column_pos" => ("minecraft:column_pos", Value::Null, ArgType::ColumnPos),
        "rotation" => ("minecraft:rotation", Value::Null, ArgType::Rotation),
        "dimension" => ("minecraft:dimension", Value::Null, ArgType::Dimension),
        "nbt" => ("minecraft:nbt_compound_tag", Value::Null, ArgType::Nbt),
        // `ResourceLocationArgument`: a free `namespace:path` of identifier characters, with no
        // registry to check. An unmodded client parses and refuses it the same way; the node
        // declares `minecraft:ask_server` (`usage_nodes`), so the identifiers it completes are the
        // mods' own (`tab_complete`).
        "id" => (
            "minecraft:resource_location",
            Value::Null,
            ArgType::Identifier,
        ),
        _ => return None,
    })
}

/// The type of the argument node that parsed `range`: the parsed node, not the value, says which
/// kind a position, id or string is.
pub(super) fn parsed_type(
    dispatcher: &Dispatcher,
    parsed_nodes: &[(usize, Range)],
    range: Range,
) -> Option<ArgType> {
    parsed_nodes.iter().find_map(|(node, parsed)| {
        if *parsed != range {
            return None;
        }
        match &dispatcher.nodes[*node].kind {
            NodeKind::Argument { ty, .. } => Some(ty.clone()),
            _ => None,
        }
    })
}

/// What a `vec3`, `vec3_uncentered`, `column_pos`, `rotation` or `block_pos` argument delivers:
/// `position` is `coordinates` resolved against the source, `rotation` the source's yaw and pitch.
pub(super) fn positional_value(
    ty: Option<&ArgType>,
    coordinates: &Coordinates,
    position: [f64; 3],
    rotation: [f32; 2],
) -> mod_api::CommandArgumentValue {
    use mod_api::CommandArgumentValue as Value;
    match ty {
        Some(ArgType::Vec3 { .. }) => Value::Vec3(position),
        Some(ArgType::ColumnPos) => {
            Value::ColumnPos([position[0].floor() as i32, position[2].floor() as i32])
        }
        Some(ArgType::Rotation) => Value::Rotation(coordinates.rotation(rotation)),
        _ => Value::BlockPos(position.map(|v| v.floor() as i32)),
    }
}

/// `DimensionArgument.getDimension`: the level of a dimension the world has loaded, or
/// `argument.dimension.invalid`.
pub(super) fn dimension_value(
    id: &str,
    level: impl FnOnce(&str) -> Option<mod_api::LevelRef>,
) -> Result<mod_api::CommandArgumentValue, CommandError> {
    level(id)
        .map(mod_api::CommandArgumentValue::Dimension)
        .ok_or_else(|| CommandError::tr("argument.dimension.invalid", &[&id]))
}

/// An `nbt` argument's compound, from the SNBT the dispatcher accepted.
pub(super) fn nbt_value(text: &str) -> Result<mod_api::CommandArgumentValue, CommandError> {
    let mut reader = Reader::new(text);
    snbt::parse_compound_argument(&mut reader).map(mod_api::CommandArgumentValue::Nbt)
}

/// The message a mod's argument parser refused with, as the engine's text. A text the
/// boundary cannot carry is reported as its own diagnostic: the refusal still fails the
/// argument, never silently passing it.
fn refused_argument_text(error: mod_api::CommandArgumentError) -> Text {
    let message: mod_api::Text = error.into();
    match message.try_into_engine(&()) {
        Ok(text) => text,
        Err(error) => Text::literal(error.to_string()),
    }
}

/// Why a mod's suggestion cannot replace part of the text `prefix` being typed: it names the
/// argument type and the suggestion, and the reason. A replacement that starts past the typed
/// text, or inside a character, has no range the chat could apply, so the suggestion is refused
/// (reported, and left out) rather than panicking on the player's input.
fn suggestion_refusal(
    id: &str,
    prefix: &str,
    suggestion: &mod_api::CommandSuggestion,
) -> Option<String> {
    let offset = suggestion.replace_start?;
    (offset > prefix.len() || !prefix.is_char_boundary(offset)).then(|| {
        format!(
            "mod command argument `{id}` refused the suggestion `{}`: it replaces from byte \
             {offset} of the {} typed",
            suggestion.text,
            prefix.len()
        )
    })
}

/// The dispatcher type of one validated tree node: a vanilla kind, or the mod's registered
/// argument type with its parser and suggestions.
pub(super) fn argument_type(node: &Value, arguments: &CommandArguments) -> ArgType {
    if node["parser"] == "minecraft:resource_key" {
        let registry = node["properties"]["registry"].as_str().unwrap_or_default();
        return ArgType::Res(
            self::registry(registry).expect("native command tree names validated registries"),
        );
    }
    if let Some(id) = node["custom_argument"].as_str() {
        let argument = arguments
            .get(id)
            .unwrap_or_else(|| panic!("native command tree names unregistered argument type {id}"));
        let parse = argument.clone();
        let suggest = argument.clone();
        let kind_id = id.to_owned();
        // Use the validated tree property rather than calling mod code again.
        let kind = if node["properties"]["type"] == "greedy" {
            horizon_client::command::CustomArgumentKind::Greedy
        } else if node[TOKEN] == true {
            horizon_client::command::CustomArgumentKind::Token
        } else {
            horizon_client::command::CustomArgumentKind::Word
        };
        return ArgType::Custom(horizon_client::command::CustomArgument::new(
            id,
            kind,
            move |input| {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parse.parse(input)))
                    .map_err(|_| Text::tr("commands.generic.failed", &[]))?
                    .map_err(refused_argument_text)
            },
            move |reg: &horizon_client::command::Registries, prefix: &str| {
                // The mod's provider decides what this sender may complete; the game keeps
                // its answer as it comes (`CommandSender::suggest`).
                let sender = reg.sender.as_ref().map(|sender| {
                    mod_api::__private::game::command_sender(
                        sender.uuid,
                        sender.name.clone(),
                        mod_api::PermissionLevel::from_level(i32::from(reg.permission)),
                    )
                });
                let Ok(results) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    suggest.suggest(sender.as_ref(), prefix)
                })) else {
                    eprintln!("mod command argument completer panicked");
                    return Vec::new();
                };
                // Suggestions use STRING_UTF8 (Java UTF-16 length). Tooltips are
                // literal NBT strings, whose modified UTF-8 byte length is unsigned u16.
                if results.iter().any(|s| {
                    s.text.encode_utf16().count() > 32767
                        || s.tooltip.as_ref().is_some_and(|tip| {
                            tip.encode_utf16()
                                .map(|unit| match unit {
                                    1..=0x7f => 1,
                                    0..=0x7ff => 2,
                                    _ => 3,
                                })
                                .sum::<usize>()
                                > 65535
                        })
                }) {
                    eprintln!("mod command argument completion exceeds the protocol string limit");
                    return Vec::new();
                }
                results
                    .into_iter()
                    .filter_map(|suggestion| {
                        // A completion for one token of the text replaces from its offset; one the
                        // player's text cannot hold is refused by name instead of panicking where
                        // the chat applies the range.
                        if let Some(refusal) = suggestion_refusal(&kind_id, prefix, &suggestion) {
                            eprintln!("{refusal}");
                            return None;
                        }
                        Some(horizon_client::command::CustomSuggestion {
                            text: suggestion.text,
                            tooltip: suggestion.tooltip,
                            replace_start: suggestion.replace_start,
                        })
                    })
                    .collect()
            },
        ));
    }
    // `player_name` asks the server for its completions (`usage_nodes`): names only, since the
    // mod gets a name and the vanilla selector completion would offer `@p` and the like.
    if node["parser"] == "minecraft:game_profile" {
        return ArgType::Custom(horizon_client::command::CustomArgument::new(
            MOD_ID,
            horizon_client::command::CustomArgumentKind::Word,
            |input| {
                if input.starts_with('@') {
                    return Err(Text::tr("argument.entity.notfound.player", &[]));
                }
                Ok(input.to_owned())
            },
            |reg: &horizon_client::command::Registries, prefix: &str| {
                let typed = prefix.to_lowercase();
                reg.players
                    .iter()
                    .filter(|name| name.to_lowercase().starts_with(&typed))
                    .map(|name| horizon_client::command::CustomSuggestion {
                        text: name.clone(),
                        tooltip: None,
                        replace_start: None,
                    })
                    .collect()
            },
        ));
    }
    if node["parser"] == "minecraft:vec3" && node[UNCENTERED] == true {
        return ArgType::Vec3 { center: false };
    }
    KINDS
        .into_iter()
        .filter_map(parser)
        .find_map(|(id, properties, ty)| {
            (node["parser"] == id && node["properties"] == properties).then_some(ty)
        })
        .expect("native command tree contains registered parsers")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_suggestion_may_replace_from_an_offset_inside_the_typed_text() {
        let whole = mod_api::CommandSuggestion::new("stone");
        assert_eq!(suggestion_refusal("test:pattern", "st", &whole), None);
        let token = mod_api::CommandSuggestion::new("stone").replacing_from(6);
        assert_eq!(suggestion_refusal("test:pattern", "stone st", &token), None);
        // Past the typed text: there is no range to replace.
        let refusal = suggestion_refusal("test:pattern", "st", &token).unwrap();
        assert!(
            refusal.contains("test:pattern"),
            "names the type: {refusal}"
        );
        assert!(
            refusal.contains("`stone`"),
            "names the suggestion: {refusal}"
        );
        // Inside a two-byte character: no boundary to split the replacement on.
        let inside = mod_api::CommandSuggestion::new("é").replacing_from(1);
        assert!(suggestion_refusal("test:pattern", "é ", &inside).is_some());
    }

    #[test]
    fn the_player_name_kind_is_vanillas_game_profile_parser() {
        let (parser, properties, ty) =
            super::parser("player_name").expect("the player_name kind has a parser");
        assert_eq!(parser, "minecraft:game_profile");
        assert_eq!(properties, Value::Null, "the parser takes no properties");
        assert!(matches!(ty, ArgType::GameProfile), "{ty:?}");
        assert!(KINDS.contains(&"player_name"));
    }

    #[test]
    fn the_id_kind_is_vanillas_free_identifier_parser() {
        let (parser, properties, ty) = super::parser("id").expect("the id kind has a parser");
        assert_eq!(parser, "minecraft:resource_location");
        assert_eq!(properties, Value::Null, "the parser takes no properties");
        assert!(matches!(ty, ArgType::Identifier), "{ty:?}");
        assert!(KINDS.contains(&"id"));
    }

    #[test]
    fn registry_names_are_resource_locations_within_their_own_budget() {
        let mut names = HashSet::new();
        for bad in [
            "a:b:c",
            "a/b:c",
            "noNamespace",
            "minecraft:",
            ":x",
            "Minecraft:x",
        ] {
            assert!(registry_in(&mut names, bad).is_err(), "{bad}");
        }
        let long = format!("a:{}", "b".repeat(MAX_REGISTRY_NAME));
        assert!(registry_in(&mut names, &long).is_err());
        let first = registry_in(&mut names, "minecraft:entity_type").unwrap();
        assert!(std::ptr::eq(
            first,
            registry_in(&mut names, "minecraft:entity_type").unwrap()
        ));
        for n in 1..MAX_REGISTRIES {
            registry_in(&mut names, &format!("mymod:r{n}")).unwrap();
        }
        let refusal = registry_in(&mut names, "mymod:one_more").unwrap_err();
        assert!(refusal.contains("past the limit"), "{refusal}");
        assert!(registry_in(&mut names, "minecraft:entity_type").is_ok());
    }
}
