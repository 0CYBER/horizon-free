/// What the Create New World screen decided, for the temporary world
/// `RealmsCreateWorldFlow.createTemporaryWorldFolder` writes before the upload.
#[derive(Clone, Debug)]
pub struct CreatedWorld {
    /// The world's level data, its name, game type, difficulty, game rules and features.
    pub level: crate::save::Level,
    pub world_gen: crate::save::WorldGen,
    pub flat_settings: Option<horizon_core::nbt::Tag>,
    /// Selected data-pack preset or editor output.
    pub dimensions: Option<horizon_core::nbt::Tag>,
    pub single_biome: Option<String>,
    /// level.dat's `DataPacks`: enabled and disabled ids.
    pub data_packs: (Vec<String>, Vec<String>),
    /// The Create World screen's temporary pack folder, moved into the world
    /// (`Files.move(tempDataPackDir, worldFolder.resolve("datapacks"))`).
    pub pack_dir: Option<PathBuf>,
}
