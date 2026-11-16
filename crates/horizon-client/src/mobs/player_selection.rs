//! A mob tick's switchable view of the players in its level.
//!
//! The views are immutable for the lifetime of a tick. Changing the selected
//! index therefore never invalidates a reference already returned to a goal.
//! Construct this only when a target-change listener needs player redirection;
//! the ordinary one-player view and tick fast path need no allocation.

use std::cell::Cell;

use super::PlayerView;

/// A real player available to the current mob tick. A `None` connection is
/// the local host; a guest is identified by its connection rather than by a
/// snapshot's position or name.
#[derive(Clone, Debug)]
pub struct TickPlayer {
    pub entity_id: u32,
    pub connection: Option<u32>,
    pub view: PlayerView,
}

/// The player the tick currently sees, selected among its stable player views.
///
/// Event routing needs [`Self::has_switched`] even if selection later returns
/// to its initial player: events emitted after the first switch must retain
/// the identity selected at emission, not the identity at the end of the tick.
#[derive(Debug)]
pub struct PlayerSelection {
    players: Vec<TickPlayer>,
    initial: Option<usize>,
    selected: Cell<Option<usize>>,
    switched: Cell<bool>,
}

impl PlayerSelection {
    /// Build a selection from the real players in this level. Duplicate entity
    /// IDs, or an initial ID that is not among the players, are invalid.
    pub fn new(players: Vec<TickPlayer>, initial: Option<u32>) -> Option<Self> {
        if players
            .iter()
            .enumerate()
            .any(|(i, p)| players[..i].iter().any(|q| q.entity_id == p.entity_id))
        {
            return None;
        }
        let selected = match initial {
            Some(id) => Some(players.iter().position(|p| p.entity_id == id)?),
            None => None,
        };
        Some(Self {
            players,
            initial: selected,
            selected: Cell::new(selected),
            switched: Cell::new(false),
        })
    }

    /// The selected player, or none when the tick sees no player.
    pub fn selected(&self) -> Option<&TickPlayer> {
        self.selected.get().map(|index| &self.players[index])
    }

    /// The player selected at the start of the tick.
    pub fn initial(&self) -> Option<&TickPlayer> {
        self.initial.map(|index| &self.players[index])
    }

    /// Find a real player in this tick's level by entity ID.
    pub fn get(&self, id: u32) -> Option<&TickPlayer> {
        self.players.iter().find(|player| player.entity_id == id)
    }

    /// Find a player riding this mob, independently of the combat target.
    pub fn rider(&self, mob_id: u32) -> Option<&TickPlayer> {
        self.players
            .iter()
            .find(|player| player.view.riding_vehicle == Some(mob_id))
    }

    /// Select a real player. An unknown or fake player's ID is refused and
    /// leaves the previous selection unchanged; callers build the input list
    /// from real players in the current level only.
    pub fn select(&self, id: u32) -> bool {
        let Some(index) = self.players.iter().position(|p| p.entity_id == id) else {
            return false;
        };
        if self.selected.replace(Some(index)) != Some(index) {
            self.switched.set(true);
        }
        true
    }

    /// Whether the selected player changed at least once during this tick.
    pub fn has_switched(&self) -> bool {
        self.switched.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::world::test_player;
    use horizon_core::geometry::Vec3;

    fn player(id: u32, conn: Option<u32>) -> TickPlayer {
        TickPlayer {
            entity_id: id,
            connection: conn,
            view: test_player(Vec3::new(f64::from(id), 64.0, 0.0)),
        }
    }

    #[test]
    fn switches_keep_borrowed_views_stable_and_track_event_routing() {
        let mut rider = player(2, Some(42));
        rider.view.riding_vehicle = Some(77);
        let players = PlayerSelection::new(vec![player(1, None), rider], Some(1)).unwrap();
        let previous = players.selected().unwrap();
        assert!(!players.has_switched());
        assert!(players.select(1));
        assert!(!players.has_switched());
        assert!(players.select(2));
        assert_eq!(previous.entity_id, 1);
        assert_eq!(previous.connection, None);
        assert_eq!(players.selected().unwrap().connection, Some(42));
        assert!(players.has_switched());
        assert!(!players.select(999));
        assert_eq!(players.selected().unwrap().entity_id, 2);
        assert!(players.select(1));
        assert!(players.has_switched());
        assert_eq!(players.initial().unwrap().entity_id, 1);
        assert_eq!(players.rider(77).unwrap().entity_id, 2);
        assert!(players.rider(88).is_none());
    }

    #[test]
    fn invalid_or_ambiguous_initial_selection_is_rejected() {
        assert!(PlayerSelection::new(vec![player(1, None)], Some(2)).is_none());
        assert!(PlayerSelection::new(vec![player(1, None), player(1, Some(4))], None).is_none());
        let no_player = PlayerSelection::new(vec![player(1, None)], None).unwrap();
        assert!(no_player.selected().is_none());
        assert!(no_player.select(1));
        assert!(no_player.has_switched());
    }
}
