//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The server host's methods of the `console` area, implemented by the game and reached through [`crate::ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait ConsoleServerHost {
    /// Side: server.
    /// Runs `line` (one leading slash is dropped) as the server's console does,
    /// from the overworld's spawn at permission level 4, as
    /// `Commands::run_command`, but keeps its feedback: `Ok((ran, lines))`,
    /// where `ran` is whether it ran without failing and `lines` are what the
    /// console would have printed for it, as plain text (`commands.time.query`
    /// and the like, an error's message), for a mod that answers a remote
    /// console (Paper's `Server.dispatchCommand` with a capturing
    /// `CommandSender`). A command that ran and failed is `Ok((false, lines))`,
    /// never a refusal. Refuses, running nothing, with
    /// `ConsoleRefusal::NotOpen` where no server is open to players and
    /// `Blank` for a line with nothing in it once its whitespace and slash are
    /// dropped. The dedicated console's own commands (`stop`, `op`,
    /// `whitelist`, `ban`...) are not reached: they answer as an unknown
    /// command, as with `run_command`. A call made while another captured
    /// command runs captures its own lines, and the outer capture goes on
    /// unchanged.
    #[access(WorldOnly)]
    fn server_run_command_captured(
        &mut self,
        line: &str,
    ) -> Result<(bool, Vec<String>), ConsoleRefusal>;
}
