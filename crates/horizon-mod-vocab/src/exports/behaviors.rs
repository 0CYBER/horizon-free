pub use entity::behaviors::{
    Activity, Behavior, BehaviorCx, BehaviorRefusal, BehaviorRegistrar, BrainMemory,
    BrainWalkTarget, MemoryStatus,
};
// hidden: the game's behavior plumbing, not part of the mod surface
#[doc(hidden)]
pub use entity::behaviors::{BehaviorCxBackend, BehaviorRegistrarBackend, RegisteredBehavior};
