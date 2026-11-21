//! `/help`.
use super::host::{CommandHost, R};
use horizon_client::{
    command::{Arg, Context, Dispatcher, Registries},
    text::Text,
};
use std::sync::Arc;

/// What `/help` reads from the game.
pub trait HelpHost: CommandHost {
    /// The session's command tree.
    fn dispatcher(&self) -> Arc<Dispatcher>;
    /// The registries the command tree parses against.
    fn registries(&self) -> &Registries;
    /// The permission level of the source.
    fn permission_level(&self) -> u8;
}

/// `/help`, with or without a command to show the usage of.
pub fn help(host: &mut dyn HelpHost, ctx: &Context) -> R<i32> {
    let d = host.dispatcher();
    let level = host.permission_level();
    match ctx.get("command") {
        None => {
            let usage = d.smart_usage(0, level);
            let n = usage.len() as i32;
            for (_, line) in usage {
                host.feedback_unbroadcast(Text::literal(format!("/{line}")));
            }
            Ok(n)
        }
        Some(Arg::Str(command)) => {
            let parse = d.parse(command, host.registries());
            if parse.context.nodes.is_empty() {
                return Err(host.error("commands.help.failed", &[]));
            }
            let last = parse.context.nodes.last().map(|n| n.0).unwrap_or(0);
            let usage = d.smart_usage(last, level);
            // HelpCommand: the prefix is the whole greedy string, and an
            // empty usage sends nothing and returns 0.
            for (_, line) in &usage {
                host.feedback_unbroadcast(Text::literal(format!("/{command} {line}")));
            }
            Ok(usage.len() as i32)
        }
        _ => Ok(0),
    }
}
