//! The argument kinds a native command usage names: the built-in ones, each a Brigadier
//! parser, and the types mods registered, with the dispatcher type each node parses as.

use super::{ArgType, Text, TryIntoEngine};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, OnceLock},
};

/// The argument kinds a native usage may name, each with a [`parser`]; `resource:<registry>`
/// and `mod_id` come on top.
pub(super) const KINDS: [&str; 12] = [
    "word",
    "text",
    "integer",
    "number",
    "boolean",
    "player",
    "entity",
    "entities",
    "players",
    "item",
    "block_pos",
    "vec3",
];

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
        "item" => ("minecraft:item_stack", Value::Null, ArgType::Item),
        "block_pos" => ("minecraft:block_pos", Value::Null, ArgType::BlockPos),
        // `Vec3Argument.vec3()`: whole x and z numbers are block centres.
        "vec3" => (
            "minecraft:vec3",
            Value::Null,
            ArgType::Vec3 { center: true },
        ),
        _ => return None,
    })
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
        // Use the validated tree property rather than calling mod code again.
        let kind = if node["properties"]["type"] == "greedy" {
            horizon_client::command::CustomArgumentKind::Greedy
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
            move |prefix| {
                let Ok(results) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    suggest.suggest(prefix)
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
                    .map(|suggestion| horizon_client::command::CustomSuggestion {
                        text: suggestion.text,
                        tooltip: suggestion.tooltip,
                    })
                    .collect()
            },
        ));
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
