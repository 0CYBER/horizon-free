//! Who an action came from or is aimed at: the player or a mob by id.

/// Who caused damage or is being targeted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Who {
    Player,
    Mob(u32),
}

/// A game event can originate from any entity, including non-living ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventSource {
    Player,
    Mob(u32),
    Entity(u32),
}

impl EventSource {
    /// Only players and mobs can participate in living-entity targeting.
    pub fn actor(self) -> Option<Who> {
        match self {
            Self::Player => Some(Who::Player),
            Self::Mob(id) => Some(Who::Mob(id)),
            Self::Entity(_) => None,
        }
    }
}

impl From<Who> for EventSource {
    fn from(who: Who) -> Self {
        match who {
            Who::Player => Self::Player,
            Who::Mob(id) => Self::Mob(id),
        }
    }
}
