/// The level the test worlds are.
pub(crate) fn here() -> LevelRef {
    LevelRef::new(WorldEpoch::new(1), crate::DimensionKey::OVERWORLD)
}
