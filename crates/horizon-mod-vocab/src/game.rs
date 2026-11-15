//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
/// Which of a column's heights (`Heightmap.Types`) a run's `y` counts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum Heightmap {
    WorldSurface,
    MotionBlocking,
    MotionBlockingNoLeaves,
    OceanFloor,
}
/// A run: `height` blocks up a column from `y` (from the column's `surface` height when one is
/// given), set to palette entry `block`, only where filter `only` matches when one is given.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct BlockRun {
    pub x: i32,
    pub z: i32,
    pub y: i32,
    pub height: u32,
    pub surface: Option<Heightmap>,
    pub block: u32,
    pub only: Option<u32>,
}
/// A bulk edit of complete typed states and partial predicates, named by run indices; NBT is not supported.
#[derive(Clone, Debug, PartialEq, Default)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct BlockEdit {
    /// Complete states to write; properties are never parsed from command strings.
    pub palette: Vec<BlockState>,
    /// Partial block or tag predicates; omitted properties are wildcards.
    pub filters: Vec<StatePredicate>,
    pub runs: Vec<BlockRun>,
}
/// The camera shake native mods ask for (`camera_shake`): turns in
/// degrees, and an offset in blocks along the camera's right, up and back.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct Shake {
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
    pub offset: [f32; 3],
}
/// `PlayerSpawnPhantomsEvent.Result`: whether the phantom spawner tries a spawn near a player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PhantomSpawnResult {
    /// Always try, skipping the skylight and sea level check.
    Allow,
    /// Try only above sea level with the sky in view.
    Default,
    /// Never try.
    Deny,
}
/// `Player.BedSleepingProblem` as `CanContinueSleepingEvent.getProblem` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum SleepProblem {
    /// The sleeper's bed is gone.
    NotPossibleHere,
    /// The bed rule no longer lets a player sleep: it is daytime.
    NotPossibleNow,
}
/// The world's clock as a client mod reads it (`clock`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct Clock {
    /// Ticks since the world was created (`Level.getGameTime`).
    pub game_time: i64,
    /// The time of day in ticks (`Level.getOverworldClockTime`): 0 at sunrise, 24000 a
    /// day. It keeps counting past 24000 (`/time add`, days going by), so take
    /// it modulo 24000 for the hour; `/time set` can make it negative.
    pub day_time: i64,
}
/// What `read_columns` read: the blocks round each column's height (`bases`, none where it isn't loaded),
/// as indexes into `palette`, a column after another, and three biome tints a column (grass, foliage, water).
#[derive(Clone, Debug, PartialEq, Default)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ColumnBlocks {
    /// Distinct complete states, including separate entries for different property combinations.
    pub palette: Vec<BlockState>,
    pub bases: Vec<Option<i32>>,
    pub blocks: Vec<u16>,
    pub tints: Vec<u32>,
}
/// Why `upgrade_block_entity_nbt` or `upgrade_entity_nbt` gave no upgraded value. Depth is counted as the data is
/// encoded (the root compound at 0): the compound the NBT writer puts around an element of a mixed list is a level
/// of its own. There is no size refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum NbtUpgradeError {
    /// The data version is below zero. Refused before any fix runs.
    NegativeVersion(i32),
    /// The data version is newer than the game's: the fixes only go forward. Refused before any fix runs.
    FutureVersion(i32),
    /// The data given has a list or compound 512 levels deep or more, the depth cap of every NBT read. Refused
    /// before any fix runs.
    TooDeep,
    /// The data given was within the cap, but the fixes made it deeper (1.9's `Riding` to `Passengers` doubles a
    /// chain's depth) and the upgraded data has a list or compound 512 levels deep or more: no NBT read, vanilla's
    /// included, could take it back. Nothing is returned.
    UpgradedTooDeep,
}
impl std::fmt::Display for NbtUpgradeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NegativeVersion(v) => write!(f, "data version {v} is negative"),
            Self::FutureVersion(v) => write!(f, "data version {v} is newer than the game's"),
            Self::TooDeep => f.write_str("the data nests 512 levels deep or more"),
            Self::UpgradedTooDeep => f.write_str("the upgraded data nests 512 levels deep or more"),
        }
    }
}
impl std::error::Error for NbtUpgradeError {}
/// Which side reads a config and when (NeoForge's `ModConfig.Type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ConfigKind {
    /// The window only, when the game starts.
    Client,
    /// Once per process: the window when the game starts, or a dedicated
    /// server when it starts. A single player's server thread shares the
    /// window's.
    Common,
    /// The server, before `server_about_to_start`, for each world it
    /// opens: the world's `serverconfig/<file>` when there is one, else
    /// `config/<file>`.
    Server,
}
impl ConfigKind {
    /// The default file name's suffix, as NeoForge's.
    fn suffix(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Common => "common",
            Self::Server => "server",
        }
    }
}
/// A config a mod registers: its kind, its file in `config/` and the text
/// written there when the file is missing.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct Config {
    pub mod_id: String,
    pub kind: ConfigKind,
    /// A path relative to `config/`: `<mod id>-<kind>.toml` unless named.
    pub file: String,
    pub default: String,
}
impl Config {
    /// `config/<mod_id>-<kind>.toml`, `default` when missing.
    pub fn new(mod_id: &str, kind: ConfigKind, default: &str) -> Self {
        Self {
            mod_id: mod_id.to_owned(),
            kind,
            file: format!("{mod_id}-{}.toml", kind.suffix()),
            default: default.to_owned(),
        }
    }
    /// The same in another file of `config/`, such as `mymod/machines.toml`.
    pub fn file(mut self, file: &str) -> Self {
        file.clone_into(&mut self.file);
        self
    }
}
/// A config's file as the game read it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ConfigFile {
    pub mod_id: String,
    pub kind: ConfigKind,
    /// The registered name, relative to `config/` (or to a world's
    /// `serverconfig/`, where that copy was read).
    pub file: String,
    pub text: String,
}
/// A data map type a mod registers (`RegisterDataMapTypesEvent`).
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct DataMapType {
    /// `mod_id:name`: not in the `minecraft` namespace. Its files are
    /// `data/<mod_id>/data_maps/<registry folder>/<name>.json`, the
    /// registry's own path for a `minecraft` registry, `<namespace>/<path>`
    /// otherwise.
    pub id: String,
    /// The registry it is for: any the game names (`minecraft:item`, `minecraft:entity_type`...).
    pub registry: String,
}
/// A mod the world was saved with that differs from the loaded mods
/// (`ModMismatchEvent`'s `MismatchedVersionInfo`).
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ModMismatch {
    /// The mod's `[mod] id`.
    pub mod_id: String,
    /// Its version when the world was saved; `""` when it wasn't known.
    pub saved_version: String,
    /// Its version now; `None` when the mod is no longer loaded.
    pub current_version: Option<String>,
}
/// An experience orb as a server mod reads it (`experience_orbs`).
/// Read by mods only.
#[derive(Clone, Debug)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ExperienceOrb {
    /// The entity's ID, as `entity_join_level` names it.
    pub id: u32,
    pub position: [f64; 3],
    /// `ExperienceOrb.value`: the signed points value saved by vanilla.
    pub value: i32,
    /// `ExperienceOrb.count`: orbs merged into this one, each worth `value`.
    pub count: u32,
}
/// What a mod's falling block does when it lands and when it hits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct Falling {
    /// `FallingBlockEntity.dropItem`: whether it drops as an item where it
    /// can't be placed.
    pub drop_item: bool,
    /// `FallingBlockEntity.hurtEntities`, at 2 a block fallen and at most 40,
    /// an anvil's.
    pub hurts: bool,
}
impl Default for Falling {
    fn default() -> Self {
        Self {
            drop_item: true,
            hurts: false,
        }
    }
}
/// A stack as a client mod's use events carry it.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct HeldItemSlot {
    pub id: ItemKey,
    pub count: u32,
    /// The inventory slot: 0 to 8, the hotbar (a mod use is the main hand's).
    pub slot: u32,
    /// `minecraft:custom_name` as SNBT, when the stack has one.
    pub custom_name: Option<String>,
    /// `minecraft:custom_data` as SNBT, when the stack has some.
    pub custom_data: Option<String>,
}
/// The block a ray along the view meets.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct Hit {
    pub block_x: i32,
    pub block_y: i32,
    pub block_z: i32,
    /// Where the ray meets the block.
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// The block's resolved identity, without state properties.
    pub block: BlockKey,
}
/// What happened to a mod's claimed item, oldest first.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ModEvent {
    /// A right-click with the item began its use: the stack, and 0 ticks.
    UseStart(HeldItemSlot, u32),
    /// The use ended: the stack it began with, and the ticks it was held. It
    /// ends when the button comes up, the hand's item changes or leaves the
    /// claim, the player respawns, or the game joins a server.
    UseStop(HeldItemSlot, u32),
}
// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// A map as a native mod sees it when `map_decorations` is asked.
// Native mods read it; nothing in Horizon does.
#[derive(Clone, Copy, Debug)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct MapInfo<'a> {
    /// The map's id (`map_<id>`).
    pub id: i32,
    /// The map's centre block `[x, z]`.
    pub center: [i32; 2],
    /// `MapItemSavedData.scale`: a map pixel is `1 << scale` blocks.
    pub scale: u8,
    /// The dimension the map shows (`minecraft:overworld`).
    pub dimension: &'a str,
    pub locked: bool,
    /// Whether it is drawn in an item frame, not held or on the cartography screen.
    pub in_frame: bool,
}
/// An icon a native mod puts on a map: the same place and pose as vanilla's
/// decorations (`MapDecoration`).
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct MapDecoration {
    /// An atlas sprite: vanilla's `map/decorations/<name>`, or a content
    /// mod's texture. An unknown sprite is left out.
    pub sprite: String,
    /// The position on the map in half map pixels, -128 (left, top) to 127.
    /// For the world position `(wx, wz)`: `(wx - center_x) / (1 << scale) * 2`,
    /// and the same for z, floored and clamped.
    pub x: i8,
    pub y: i8,
    /// 0 to 15, in 22.5 degree steps clockwise (only the low four bits count).
    pub rot: u8,
    /// Whether it shows in an item frame and on the cartography screen
    /// (`MapDecorationType.showOnItemFrame`); a held map shows all.
    pub shown_on_frame: bool,
}
// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// One mob's frame, as `EntityRenderer.extractRenderState` fills a
/// `LivingEntityRenderState`: what a model reads to pose and animate itself.
/// Angles are in degrees, interpolated to the frame's partial tick.
// Native mods read it; nothing in Horizon does.
#[derive(Clone, Debug)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct MobRender<'a> {
    /// The mob's network id, as the mob helpers and mod messages name it.
    pub id: u32,
    /// Its entity type (`minecraft:villager`).
    pub kind: &'a str,
    /// Its `CustomName` as plain text, when it has one.
    pub custom_name: Option<&'a str>,
    /// `isBaby`.
    pub baby: bool,
    /// The frame's partial tick, 0 to 1.
    pub partial: f32,
    /// `ageInTicks`: ticks lived plus the partial tick, for idle motion.
    pub age: f32,
    /// `bodyRot`: the way the body faces (0 is south, 90 west).
    pub body_yaw: f32,
    /// `yRot`: the head's turn from the body, -180 to 180.
    pub head_yaw: f32,
    /// `xRot`: the head's pitch, positive looking down.
    pub pitch: f32,
    /// `walkAnimationPos`: how far the legs have swung, for a walk cycle
    /// (a vanilla quadruped's legs swing `cos(pos * 0.6662) * 1.4 * speed` radians).
    pub walk_position: f32,
    /// `walkAnimationSpeed`: how fast it walks, 0 standing to about 1.
    pub walk_speed: f32,
    /// Ticks left of the red hurt flash (10 when hit).
    pub hurt_time: i32,
    /// Ticks since it died, 0 while alive (vanilla tips the body over 20).
    pub death_time: i32,
    pub on_ground: bool,
}
// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// Which player a [`MobRender`] is, for `render_player_pre` and
/// `render_player_post`.
// Native mods read it; nothing in Horizon does.
#[derive(Clone, Copy, Debug)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct PlayerRender<'a> {
    /// The player's profile name.
    pub name: &'a str,
    /// Whether it is the player of this window, drawn in third person.
    pub local: bool,
}
// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// One face of a `native_draw_cube` box: its sprite (`mymod:entity/cog`,
/// `mymod:block/casing`) and its UVs in texels of 16 per side, as a model
/// face's `uv` (none takes the box's own, as a model's default).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct CubeFace<'a> {
    pub sprite: &'a str,
    pub uv: Option<[f32; 4]>,
}
impl<'a> CubeFace<'a> {
    /// A face of `sprite` with the box's own UVs.
    pub const fn new(sprite: &'a str) -> Self {
        Self { sprite, uv: None }
    }
    /// The same face with UVs `uv`, `[u0, v0, u1, v1]` in texels of 16 per side.
    pub const fn with_uv(self, uv: [f32; 4]) -> Self {
        Self {
            uv: Some(uv),
            ..self
        }
    }
}

#[doc(hidden)]
pub(crate) mod built {
    #[allow(unused_imports)]
    use super::*;
    // exhaustive: the game builds it with a literal; mods only read it
    #[derive(horizon_mod_api_macros::Wire)]
    pub struct ConfigFile {
        pub mod_id: String,
        pub kind: ConfigKind,
        pub file: String,
        pub text: String,
    }
    impl From<ConfigFile> for super::ConfigFile {
        fn from(v: ConfigFile) -> Self {
            let ConfigFile {
                mod_id,
                kind,
                file,
                text,
            } = v;
            Self {
                mod_id,
                kind,
                file,
                text,
            }
        }
    }
}

/// Saved-world resolution records and listener-boundary attribution.
#[path = "mismatch_resolution.rs"]
pub mod mismatch_resolution;
pub use mismatch_resolution::ModMismatchResolution;
