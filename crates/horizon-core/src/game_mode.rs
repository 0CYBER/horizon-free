//! The game modes shared by command parsing, gameplay and the mod API.

/// A player's game mode (vanilla's `GameType`).
// exhaustive: GameType is the fixed set of four vanilla modes, not an extensible registry
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GameMode {
    /// Survival mode.
    Survival,
    /// Creative mode.
    Creative,
    /// Adventure mode.
    Adventure,
    /// Spectator mode.
    Spectator,
}

impl GameMode {
    /// Every mode, in id order.
    pub const ALL: [Self; 4] = [
        Self::Survival,
        Self::Creative,
        Self::Adventure,
        Self::Spectator,
    ];

    /// `GameType.getId`: 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub fn id(self) -> i32 {
        match self {
            Self::Survival => 0,
            Self::Creative => 1,
            Self::Adventure => 2,
            Self::Spectator => 3,
        }
    }

    /// The mode with `GameType` id `id`, or `None` for an unknown id.
    pub fn from_id(id: i32) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.id() == id)
    }

    /// `GameType.getName`: `survival`, `creative`, `adventure`, `spectator`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Survival => "survival",
            Self::Creative => "creative",
            Self::Adventure => "adventure",
            Self::Spectator => "spectator",
        }
    }

    /// The mode with the exact name accepted by `GameModeArgument`.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.name() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::GameMode;

    #[test]
    fn game_mode_names_and_ids_match_game_type() {
        for (mode, id, name) in [
            (GameMode::Survival, 0, "survival"),
            (GameMode::Creative, 1, "creative"),
            (GameMode::Adventure, 2, "adventure"),
            (GameMode::Spectator, 3, "spectator"),
        ] {
            assert_eq!(mode.id(), id);
            assert_eq!(mode.name(), name);
            assert_eq!(GameMode::from_id(id), Some(mode));
            assert_eq!(GameMode::from_name(name), Some(mode));
        }
        for id in [-1, 4, i32::MIN, i32::MAX] {
            assert_eq!(GameMode::from_id(id), None);
        }
        for name in ["Creative", "1", "", "survival "] {
            assert_eq!(GameMode::from_name(name), None);
        }
    }
}
