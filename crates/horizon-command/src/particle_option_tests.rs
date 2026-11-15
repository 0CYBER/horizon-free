//! `ParticleArgument.readParticle` decodes the options through the particle
//! type's codec, so a value the codec refuses fails the argument with
//! `particle.invalidOptions` instead of reaching the command.
use super::args::{Registries, RegistryIds};
use super::vanilla;

fn registries() -> Registries {
    let mut r = Registries {
        permission: 4,
        ..Registries::default()
    };
    r.by_registry.insert(
        "minecraft:item".into(),
        RegistryIds {
            ids: vec!["minecraft:stone".into()],
            tags: Vec::new(),
        },
    );
    r
}

/// The exception key and its argument (`particle.invalidOptions`'s codec
/// message) the line fails to parse with, if any. The test process has no
/// language table, so the message is the component's argument.
fn error(line: &str) -> Option<(String, String)> {
    let reg = registries();
    let parse = vanilla::dispatcher().parse(line, &reg);
    parse.executable().err().map(|e| {
        let argument = e.message.component()["with"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        (e.key, argument)
    })
}

#[test]
fn particle_options_the_codecs_refuse_fail_the_argument() {
    // The codec's own messages, with the fields' errors joined last to first
    // (`RecordCodecBuilder`).
    for (line, message) in [
        // ScalableParticleOptionsBase.SCALE: [0.01; 4.0].
        (
            "particle minecraft:dust{color:0,scale:10}",
            "Value must be within range [0.01;4.0]: 10.0",
        ),
        (
            "particle minecraft:dust{color:0,scale:0.009}",
            "Value must be within range [0.01;4.0]: 0.009",
        ),
        (
            "particle minecraft:dust{color:0,scale:\"x\"}",
            "Not a number",
        ),
        (
            "particle minecraft:dust_color_transition{from_color:0,to_color:0,scale:10}",
            "Value must be within range [0.01;4.0]: 10.0",
        ),
        // ExtraCodecs.RGB_COLOR_CODEC: an int or three floats.
        (
            "particle minecraft:dust{color:\"red\",scale:1}",
            "Failed to parse either. First: Not a number; Second: Not a list: \"red\"",
        ),
        (
            "particle minecraft:dust{color:[1,2,3,4],scale:1}",
            "Input is not a list of 3 elements",
        ),
        (
            "particle minecraft:dust{color:[1,\"a\",2],scale:1}",
            "Failed to parse either. First: Not a number; Second: Not a number; Input is not a list of 3 elements",
        ),
        // Every field's error, joined last to first.
        (
            "particle minecraft:dust{color:\"red\",scale:10}",
            "Value must be within range [0.01;4.0]: 10.0; Failed to parse either. First: Not a number; Second: Not a list: \"red\"",
        ),
        (
            "particle minecraft:dust{color:\"red\"}",
            "No key scale in MapLike[{color:\"red\"}]; Failed to parse either. First: Not a number; Second: Not a list: \"red\"",
        ),
        (
            "particle minecraft:dust_color_transition{from_color:\"red\",to_color:\"x\",scale:10}",
            "Value must be within range [0.01;4.0]: 10.0; Failed to parse either. First: Not a number; Second: Not a list: \"x\"; Failed to parse either. First: Not a number; Second: Not a list: \"red\"",
        ),
        // ItemStackTemplate.CODEC: the item must be in the registry.
        (
            "particle minecraft:item{item:\"minecraft:not_an_item\"}",
            "Failed to parse either. First: Not a map: \"minecraft:not_an_item\"; Second: Unknown registry key in ResourceKey[minecraft:root / minecraft:item]: minecraft:not_an_item",
        ),
        (
            "particle minecraft:item{item:{}}",
            "Failed to parse either. First: No key id in MapLike[{}]; Second: Not a string",
        ),
        (
            "particle minecraft:item{item:{id:\"minecraft:not_an_item\"}}",
            "Failed to parse either. First: Unknown registry key in ResourceKey[minecraft:root / minecraft:item]: minecraft:not_an_item; Second: Not a string",
        ),
        // A `count` error on its own keeps the record's partial value.
        (
            "particle minecraft:item{item:{id:\"minecraft:stone\",count:0}}",
            "Value must be within range [1;99]: 0",
        ),
        (
            "particle minecraft:item{item:{id:\"minecraft:stone\",count:\"x\"}}",
            "Not a number",
        ),
        (
            "particle minecraft:item{item:{id:\"minecraft:not_an_item\",count:0}}",
            "Failed to parse either. First: Value must be within range [1;99]: 0; Unknown registry key in ResourceKey[minecraft:root / minecraft:item]: minecraft:not_an_item; Second: Not a string",
        ),
    ] {
        assert_eq!(
            error(line)
                .as_ref()
                .map(|(key, argument)| (key.as_str(), argument.as_str())),
            Some(("particle.invalidOptions", message)),
            "{line}"
        );
    }
}

#[test]
fn particle_options_the_codecs_accept_still_parse() {
    for line in [
        "particle minecraft:dust{color:0,scale:0.01}",
        "particle minecraft:dust{color:0,scale:4.0}",
        "particle minecraft:dust{color:[1.0,0.0,0.0],scale:2.0}",
        "particle minecraft:dust{color:[1,2,3],scale:1}",
        "particle minecraft:dust{color:1.5,scale:1}",
        "particle minecraft:dust_color_transition{from_color:0,to_color:[1,1,1],scale:1}",
        "particle minecraft:item{item:\"minecraft:stone\"}",
        "particle minecraft:item{item:\"stone\"}",
        "particle minecraft:item{item:{id:\"minecraft:stone\"}}",
        "particle minecraft:item{item:{id:\"minecraft:stone\",count:5}}",
        "particle minecraft:item{item:{id:\"minecraft:stone\",count:1.5}}",
        "particle minecraft:block{block_state:\"minecraft:stone\"}",
    ] {
        assert_eq!(error(line), None, "{line}");
    }
}
