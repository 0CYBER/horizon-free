//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The server host's methods of the `scoreboard` area, implemented by the game and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait ScoreboardServerHost {
    /// Side: server.
    /// The team `name` (`Scoreboard.getPlayerTeam`), or None when there is none, for a name that
    /// is empty or has a space, and on a client. Teams are kept with the world's scoreboard,
    /// which the server saves.
    #[access(WorldOnly)]
    fn team(&self, name: &str) -> Option<TeamRef>;
    /// Side: server.
    /// `Scoreboard.addPlayerTeam`, as `/team add`: a new team with no colour and no members.
    /// `NameTaken` when a team of that name exists; `Unavailable` on a client and for a name
    /// that is empty, has a space, or one the saved scoreboard could not keep (its modified
    /// UTF-8 form is over 65535 bytes).
    #[access(WorldOnly)]
    fn add_team(&mut self, name: &str) -> Result<TeamRef, TeamError>;
    /// Side: server.
    /// `PlayerTeam.setColor`, as `/team modify <team> color`: the team's members' names take
    /// the colour in every client from the next tick's scoreboard sync, and None (`reset`)
    /// takes it away. True when the colour changed; false for a stale team, on a client and
    /// when the team already has that colour.
    #[access(WorldOnly)]
    fn team_set_color(&mut self, team: TeamRef, color: Option<TeamColor>) -> bool;
    /// Side: server.
    /// `Scoreboard.addPlayerToTeam`, as `/team join`: `holder` leaves the team it is on and
    /// joins `team`, in every client from the next tick's sync. Ok(false) when it is on that
    /// team already. `Stale` for a team that is gone, `UnknownHolder` for a holder the
    /// scoreboard cannot key (an empty name, one the saved scoreboard could not keep, a stale
    /// player or entity), `Unavailable` on a client.
    #[access(WorldOnly)]
    fn team_join(&mut self, team: TeamRef, holder: ScoreHolder) -> Result<bool, TeamError>;
    /// Side: server.
    /// `Scoreboard.removePlayerFromTeam`, as `/team leave`: `holder` leaves `team`. False for
    /// a stale team, a holder that is not on it, and on a client.
    #[access(WorldOnly)]
    fn team_leave(&mut self, team: TeamRef, holder: ScoreHolder) -> bool;
    /// Side: server.
    /// `Scoreboard.removePlayerTeam`, as `/team remove`: the team is gone and its members have
    /// no team. False for a stale team and on a client.
    #[access(WorldOnly)]
    fn team_remove(&mut self, team: TeamRef) -> bool;
}
