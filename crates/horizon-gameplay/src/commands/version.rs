//! `/version`.
use super::host::{CommandHost, R};
use horizon_client::text::Text;
use horizon_core::version::{BUILD_TIME, DATA_VERSION, STABLE, VANILLA_VERSION, VERSION_ID};

/// What `/version` needs from the game: whether its source is silent, and the chat.
pub trait VersionHost: CommandHost {
    /// Whether the command's source is silent (`CommandSourceStack.silent`).
    fn source_silent(&self) -> bool;
    /// A system message in the chat of the player.
    fn chat_system_message(&mut self, text: Text);
    /// `VersionCommand.dumpVersion`'s lines.
    fn version_lines(&self) -> Vec<Text>;
}

fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}

/// `VersionCommand.dumpVersion`'s lines: Horizon reports the version it
/// implements. `protocol` is the network protocol version.
pub fn version_lines(protocol: i32) -> Vec<Text> {
    let mut lines: Vec<Text> = [
        ("commands.version.id", vec![Text::literal(VERSION_ID)]),
        (
            "commands.version.name",
            vec![Text::literal(VANILLA_VERSION)],
        ),
        ("commands.version.data", vec![num(DATA_VERSION)]),
        ("commands.version.series", vec![Text::literal("main")]),
        (
            "commands.version.protocol",
            vec![num(protocol), Text::literal(format!("0x{protocol:x}"))],
        ),
        (
            "commands.version.build_time",
            vec![Text::literal(BUILD_TIME)],
        ),
        (
            "commands.version.pack.resource",
            vec![Text::literal("97.1")],
        ),
        ("commands.version.pack.data", vec![Text::literal("121.0")]),
    ]
    .into_iter()
    .map(|(key, args)| Text::translate(key, &args))
    .collect();
    lines.push(Text::translate(
        if STABLE {
            "commands.version.stable.yes"
        } else {
            "commands.version.stable.no"
        },
        &[],
    ));
    lines
}

/// `VersionCommand`.
pub fn version(host: &mut dyn VersionHost) -> R<i32> {
    if !host.source_silent() {
        host.chat_system_message(Text::translate("commands.version.header", &[]));
        for line in host.version_lines() {
            host.chat_system_message(line);
        }
    }
    Ok(1)
}
