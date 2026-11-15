// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod game;
pub use crate::game::NbtUpgradeError;
pub use crate::game::{
    BlockEdit, BlockRun, Clock, ColumnBlocks, Config, ConfigFile, ConfigKind, CubeFace,
    DataMapType, ExperienceOrb, Falling, Heightmap, HeldItemSlot, Hit, MapDecoration, MapInfo,
    MobRender, ModEvent, ModMismatch, ModMismatchResolution, PhantomSpawnResult, PlayerRender,
    Shake, SleepProblem,
};
mod __private {
    mod built {
        pub use crate::game::built::ConfigFile;
    }
}
