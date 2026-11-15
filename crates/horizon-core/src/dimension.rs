#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dimension {
    Overworld,
    Nether,
    End,
    /// A level past vanilla's three, from a data pack or a mod.
    Custom(LevelKey),
}

use crate::LevelKey;
impl Dimension {
    /// The level key of this dimension.
    pub fn key(self) -> LevelKey {
        match self {
            Self::Overworld => LevelKey::OVERWORLD,
            Self::Nether => LevelKey::NETHER,
            Self::End => LevelKey::END,
            Self::Custom(key) => key,
        }
    }

    /// The dimension of a level key.
    pub fn of_level(key: LevelKey) -> Self {
        match key {
            LevelKey::OVERWORLD => Self::Overworld,
            LevelKey::NETHER => Self::Nether,
            LevelKey::END => Self::End,
            _ => Self::Custom(key),
        }
    }
}
