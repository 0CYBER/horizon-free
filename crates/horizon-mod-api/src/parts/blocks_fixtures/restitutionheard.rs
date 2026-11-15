/// What a restitution part heard: whether its level was a client's, the entity, its type,
/// the block's position, and whether the level host reads the block's state there.
// no wire: test-only observation log shared with the fixture, never passed to a mod.
pub type RestitutionHeard =
    std::sync::Arc<std::sync::Mutex<Vec<(bool, i32, String, BlockPos, bool)>>>;
