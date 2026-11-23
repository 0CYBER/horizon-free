//! Scoreboard teams for server mods (`/team add|join|leave|remove|modify color`).
use super::Game;
use mod_api::__private::game::{Fields, team_ref};
use mod_api::{ScoreHolder, TeamColor, TeamError, TeamRef};

/// Whether the scoreboard's save keeps `text` as it is: the disk writer empties an NBT string whose modified
/// UTF-8 form (U+0000 as two bytes, a character past the basic plane as a surrogate pair of three bytes each)
/// exceeds 65535 bytes, so a team or a holder named by it could not be loaded under that name; and the
/// team packet's own limit of 32767 UTF-16 units.
fn saves_intact(text: &str) -> bool {
    // The team packet writes a name and a holder as strings of at most 32767 UTF-16 units; a longer one is
    // refused by the client, which then misses the team or the membership.
    if text.encode_utf16().count() > 32_767 {
        return false;
    }
    let encoded: usize = text
        .chars()
        .map(|c| match c as u32 {
            0 => 2,
            1..=0x7f => 1,
            0x80..=0x7ff => 2,
            0x800..=0xffff => 3,
            _ => 6,
        })
        .sum();
    encoded <= usize::from(u16::MAX)
}

/// The chat colour name `PlayerTeam.setColor` and the scoreboard keep.
fn color_name(color: TeamColor) -> &'static str {
    match color {
        TeamColor::Black => "black",
        TeamColor::DarkBlue => "dark_blue",
        TeamColor::DarkGreen => "dark_green",
        TeamColor::DarkAqua => "dark_aqua",
        TeamColor::DarkRed => "dark_red",
        TeamColor::DarkPurple => "dark_purple",
        TeamColor::Gold => "gold",
        TeamColor::Gray => "gray",
        TeamColor::DarkGray => "dark_gray",
        TeamColor::Blue => "blue",
        TeamColor::Green => "green",
        TeamColor::Aqua => "aqua",
        TeamColor::Red => "red",
        TeamColor::LightPurple => "light_purple",
        TeamColor::Yellow => "yellow",
        TeamColor::White => "white",
        _ => "white",
    }
}

impl Game<'_> {
    /// The name of a team handle that still names a team: of this open of the world and on a server.
    fn live_team(&mut self, team: &TeamRef) -> Option<String> {
        if self.remote.is_some() || self.native_lifecycle.epoch()? != team.fields().epoch {
            return None;
        }
        self.load_scoreboard();
        let name = team.fields().name;
        self.score.board.team(name).map(|_| name.to_owned())
    }
    /// What the scoreboard keys a holder by: a player's name, an entity's UUID, a name as written.
    fn holder_key(&mut self, holder: &ScoreHolder) -> Option<String> {
        let key = match holder {
            ScoreHolder::Player(player) => self.player_name(*player)?,
            ScoreHolder::Entity(entity) => {
                horizon_protocol::crypto::uuid_string(self.entity_uuid(*entity)?)
            }
            ScoreHolder::Name(name) => name.clone(),
            _ => return None,
        };
        (!key.is_empty() && saves_intact(&key)).then_some(key)
    }

    /// `Scoreboard.getPlayerTeam`: the team of that name, or None.
    pub(super) fn team(&self, name: &str) -> Option<TeamRef> {
        if self.remote.is_some() {
            return None;
        }
        let epoch = self.native_lifecycle.epoch()?;
        // The board loads with the world; a team no one has touched yet is read from it first.
        self.score.board.team(name)?;
        Some(team_ref(epoch, name.to_owned()))
    }
    /// `Scoreboard.addPlayerTeam`: a new team with no colour. `NameTaken` when it exists;
    /// `Unavailable` on a client and for a name that is empty, has a space or is one the saved
    /// scoreboard could not keep.
    pub(super) fn add_team(&mut self, name: &str) -> Result<TeamRef, TeamError> {
        if self.remote.is_some()
            || name.is_empty()
            || name.contains(char::is_whitespace)
            || !saves_intact(name)
        {
            return Err(TeamError::Unavailable);
        }
        let epoch = self
            .native_lifecycle
            .epoch()
            .ok_or(TeamError::Unavailable)?;
        self.load_scoreboard();
        if self.score.board.team(name).is_some() {
            return Err(TeamError::NameTaken);
        }
        self.score.board.add_team(name);
        Ok(team_ref(epoch, name.to_owned()))
    }
    /// `PlayerTeam.setColor`: true when the colour changed.
    pub(super) fn team_set_color(&mut self, team: TeamRef, color: Option<TeamColor>) -> bool {
        let Some(name) = self.live_team(&team) else {
            return false;
        };
        let color = color.map(|color| color_name(color).to_owned());
        let Some(existing) = self.score.board.team(&name) else {
            return false;
        };
        if existing.color == color {
            return false;
        }
        self.score.board.add_team(&name).color = color;
        true
    }
    /// `Scoreboard.addPlayerToTeam`: `holder` leaves its team and joins `team`; Ok(false) when it
    /// is on that team already.
    pub(super) fn team_join(
        &mut self,
        team: TeamRef,
        holder: ScoreHolder,
    ) -> Result<bool, TeamError> {
        if self.remote.is_some() {
            return Err(TeamError::Unavailable);
        }
        let name = self.live_team(&team).ok_or(TeamError::Stale)?;
        let key = self.holder_key(&holder).ok_or(TeamError::UnknownHolder)?;
        if self
            .score
            .board
            .players_team(&key)
            .is_some_and(|current| current.name == name)
        {
            return Ok(false);
        }
        self.score.board.add_to_team(&key, &name);
        Ok(true)
    }
    /// `Scoreboard.removePlayerFromTeam`: true when `holder` was on `team`.
    pub(super) fn team_leave(&mut self, team: TeamRef, holder: ScoreHolder) -> bool {
        let Some(name) = self.live_team(&team) else {
            return false;
        };
        let Some(key) = self.holder_key(&holder) else {
            return false;
        };
        if self
            .score
            .board
            .players_team(&key)
            .is_none_or(|current| current.name != name)
        {
            return false;
        }
        self.score.board.remove_from_team(&key)
    }
    /// `Scoreboard.removePlayerTeam`: true when the team existed.
    pub(super) fn team_remove(&mut self, team: TeamRef) -> bool {
        let Some(name) = self.live_team(&team) else {
            return false;
        };
        self.score.board.remove_team(&name);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;
    use mod_api::{ScoreHolder, TeamColor, TeamError};

    /// Teams are made, coloured, joined, left and removed in the world's scoreboard, which
    /// the next tick's sync sends to the players; refused input changes nothing.
    #[test]
    #[ignore = "daily"]
    fn teams_are_kept_in_the_worlds_scoreboard() {
        with_guests(|game| {
            game.load_scoreboard();
            let red = game.add_team("red").unwrap();
            assert_eq!(game.add_team("red"), Err(TeamError::NameTaken));
            assert_eq!(game.team("red"), Some(red.clone()));
            assert!(game.team("blue").is_none());
            assert_eq!(game.add_team("two words"), Err(TeamError::Unavailable));
            // a name the packet or the saved scoreboard could not carry is refused; one at the limit is kept
            assert_eq!(
                game.add_team(&"a".repeat(32_768)),
                Err(TeamError::Unavailable)
            );
            assert_eq!(
                game.add_team(&"\u{1F600}".repeat(16_384)),
                Err(TeamError::Unavailable)
            );
            assert!(game.add_team(&"b".repeat(32_767)).is_ok());
            // colours: a change is true, the same colour again is not
            assert!(game.team_set_color(red.clone(), Some(TeamColor::Red)));
            assert_eq!(
                game.score.board.team("red").unwrap().color.as_deref(),
                Some("red")
            );
            assert!(!game.team_set_color(red.clone(), Some(TeamColor::Red)));
            assert!(game.team_set_color(red.clone(), None));
            assert_eq!(game.score.board.team("red").unwrap().color, None);
            // holders
            let guest = ScoreHolder::Name("guest12".into());
            assert_eq!(game.team_join(red.clone(), guest.clone()), Ok(true));
            assert_eq!(
                game.team_join(red.clone(), guest.clone()),
                Ok(false),
                "already on that team"
            );
            let a = game.player(12).unwrap();
            assert_eq!(
                game.team_join(red.clone(), ScoreHolder::Player(a)),
                Ok(false),
                "a player is keyed by name"
            );
            for bad in ["", &"h".repeat(32_768)] {
                assert_eq!(
                    game.team_join(red.clone(), ScoreHolder::Name(bad.to_owned())),
                    Err(TeamError::UnknownHolder)
                );
            }
            assert_eq!(
                game.score.board.players_team("guest12").unwrap().name,
                "red"
            );
            assert!(!game.team_leave(red.clone(), ScoreHolder::Name("guest13".into())));
            assert!(game.team_leave(red.clone(), guest.clone()));
            assert!(!game.team_leave(red.clone(), guest));
            assert_eq!(
                game.team_join(red.clone(), ScoreHolder::Name("guest13".into())),
                Ok(true)
            );
            assert!(game.team_remove(red.clone()));
            assert!(game.score.board.players_team("guest13").is_none());
            assert!(!game.team_remove(red.clone()), "a removed team is stale");
            assert_eq!(
                game.team_join(red, ScoreHolder::Name("guest13".into())),
                Err(TeamError::Stale)
            );
        });
    }
}
