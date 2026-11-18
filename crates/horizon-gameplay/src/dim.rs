//! The open world's dimensions: vanilla's three and those a data pack or a mod adds.

/// The open world's dimensions past vanilla's three. One per process, as a window
/// and its server thread name the same dimensions.
static EXTRA_DIMS: std::sync::RwLock<Vec<mod_api::DimensionKey>> =
    std::sync::RwLock::new(Vec::new());

/// The extra dimensions as they were at [`Dim::snapshot_extras`], for a test to put back.
/// Opaque, so a crate can't put a key there that a [`Dim`] variant already names.
#[doc(hidden)]
pub struct ExtraDimsSnapshot(Vec<mod_api::DimensionKey>);

/// The table of extra dimensions held for writing, for a test that checks a reader doesn't wait on it.
#[doc(hidden)]
pub struct ExtraDimsHeld(
    #[allow(dead_code)] std::sync::RwLockWriteGuard<'static, Vec<mod_api::DimensionKey>>,
);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dim {
    Overworld,
    Nether,
    End,
    /// A dimension past vanilla's three, from a data pack or a mod.
    Extra(mod_api::DimensionKey),
}
impl Dim {
    /// The level as block behaviour and the mods name it.
    pub fn level(self) -> horizon_core::dimension::Dimension {
        match self {
            Self::Overworld => horizon_core::dimension::Dimension::Overworld,
            Self::Nether => horizon_core::dimension::Dimension::Nether,
            Self::End => horizon_core::dimension::Dimension::End,
            Self::Extra(_) => horizon_core::dimension::Dimension::of_level(
                horizon_core::LevelKey::parse(self.id())
                    .unwrap_or(horizon_core::LevelKey::OVERWORLD),
            ),
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Overworld => "minecraft:overworld",
            Self::Nether => "minecraft:the_nether",
            Self::End => "minecraft:the_end",
            Self::Extra(key) => {
                mod_api::__private::dimension_id(key).expect("the game's table made the key")
            }
        }
    }
    pub fn from_id(id: &str) -> Self {
        match id {
            "minecraft:overworld" => Self::Overworld,
            "minecraft:the_nether" => Self::Nether,
            "minecraft:the_end" => Self::End,
            _ => Self::extra(id).unwrap_or(Self::Overworld),
        }
    }
    /// `Level.getSeaLevel` of the level: the sea level its noise settings give a mod's level, else
    /// the built-in level's.
    pub fn sea_level(self) -> i32 {
        Self::sea_level_of_id(self.id())
    }
    /// [`Self::sea_level`] of the level `id` names.
    pub fn sea_level_of_id(id: &str) -> i32 {
        horizon_core::LevelKey::parse(id).map_or(63, horizon_world::level::sea_level_of)
    }
    /// The open world's dimension `id` past vanilla's three.
    pub fn extra(id: &str) -> Option<Self> {
        let extra = EXTRA_DIMS.read().unwrap();
        let key = extra
            .iter()
            .find(|key| mod_api::__private::dimension_id(**key) == Some(id))?;
        Some(Self::Extra(*key))
    }
    pub fn is_vanilla(id: &str) -> bool {
        matches!(
            id,
            "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end"
        )
    }
    /// Whether the open world has the dimension `id`.
    pub fn exists(id: &str) -> bool {
        Self::is_vanilla(id) || Self::extra(id).is_some()
    }
    /// The open world's dimensions, in the order its server ticks them.
    pub fn all() -> Vec<Self> {
        let extra = EXTRA_DIMS.read().unwrap();
        [Self::Overworld, Self::Nether, Self::End]
            .into_iter()
            .chain(extra.iter().copied().map(Self::Extra))
            .collect()
    }
    /// The dimensions past vanilla's three of the world being opened. A vanilla id is skipped:
    /// it already has its own variant, and listing it here would make [`Self::all`] name its level twice.
    pub fn set_extra<'a>(ids: impl Iterator<Item = &'a str>) {
        *EXTRA_DIMS.write().unwrap() = ids
            .filter(|id| !Self::is_vanilla(id))
            .map(mod_api::__private::dimension_key)
            .collect();
    }
    /// Adds the dimension `id` a remote server sent, which no local world told of, so that
    /// [`Self::from_id`] finds it.
    pub fn add_extra(id: &str) {
        if !Self::exists(id) {
            EXTRA_DIMS
                .write()
                .unwrap()
                .push(mod_api::__private::dimension_key(id));
        }
    }
    /// The dimension as mods' level handles name it.
    pub fn key(self) -> mod_api::DimensionKey {
        match self {
            Self::Overworld => mod_api::DimensionKey::OVERWORLD,
            Self::Nether => mod_api::DimensionKey::NETHER,
            Self::End => mod_api::DimensionKey::END,
            Self::Extra(key) => key,
        }
    }
    /// The level's shape in a world of vanilla's dimension types.
    pub fn shape(self) -> horizon_client::save::LevelShape {
        use horizon_client::save::LevelShape;
        match self {
            Self::Overworld => LevelShape::OVERWORLD,
            Self::Nether => LevelShape::NETHER,
            Self::End => LevelShape::END,
            Self::Extra(_) => LevelShape::OVERWORLD,
        }
    }
    /// The level the world has for `level`; None for a `Custom` level it lacks.
    pub fn from_level(level: horizon_core::dimension::Dimension) -> Option<Self> {
        use horizon_core::dimension::Dimension as LevelDimension;
        Some(match level {
            LevelDimension::Overworld => Self::Overworld,
            LevelDimension::Nether => Self::Nether,
            LevelDimension::End => Self::End,
            LevelDimension::Custom(key) => {
                Self::exists(key.id()).then(|| Self::from_id(key.id()))?
            }
        })
    }
    /// Drops the dimension `id` that [`Self::add_extra`] added: its level is closed, or a remote
    /// For tests: the extra dimensions now, to put back with [`Self::restore_extras`].
    #[doc(hidden)]
    pub fn snapshot_extras() -> ExtraDimsSnapshot {
        ExtraDimsSnapshot(EXTRA_DIMS.read().unwrap().clone())
    }
    /// For tests: puts back the extra dimensions [`Self::snapshot_extras`] saw.
    #[doc(hidden)]
    pub fn restore_extras(snapshot: ExtraDimsSnapshot) {
        *EXTRA_DIMS.write().unwrap() = snapshot.0;
    }
    /// For tests: holds the table of extra dimensions for writing until the result drops.
    #[doc(hidden)]
    pub fn hold_extras() -> ExtraDimsHeld {
        ExtraDimsHeld(EXTRA_DIMS.write().unwrap())
    }
    /// server's level that its client left.
    pub fn remove_extra(id: &str) {
        EXTRA_DIMS
            .write()
            .unwrap()
            .retain(|key| mod_api::__private::dimension_id(*key) != Some(id));
    }
}
