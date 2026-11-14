//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
/// `PathComputationType`: what a path is computed for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum PathComputationType {
    Land,
    Water,
    Air,
}
impl PathComputationType {
    /// Every kind, in vanilla's order.
    pub const ALL: [Self; 3] = [Self::Land, Self::Water, Self::Air];
}
/// `PathType`: what a pathfinding node is for a mob, each with a default
/// malus a mob may override (`Mob.setPathfindingMalus`). Vanilla 26.3's set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum PathType {
    Blocked,
    Open,
    Walkable,
    WalkableDoor,
    Trapdoor,
    PowderSnow,
    OnTopOfPowderSnow,
    Fence,
    Lava,
    Water,
    WaterBorder,
    Rail,
    UnpassableRail,
    FireInNeighbor,
    Fire,
    DamagingInNeighbor,
    Damaging,
    DoorOpen,
    DoorWoodClosed,
    DoorIronClosed,
    Breach,
    Leaves,
    StickyHoney,
    Cocoa,
    DamageCautious,
    OnTopOfTrapdoor,
    BigMobsCloseToDanger,
}
impl PathType {
    /// How many kinds there are: the length of a per-kind table.
    pub const COUNT: usize = 27;
    /// Every kind, in declaration order: `ALL[kind as usize] == kind`.
    pub const ALL: [Self; Self::COUNT] = [
        Self::Blocked,
        Self::Open,
        Self::Walkable,
        Self::WalkableDoor,
        Self::Trapdoor,
        Self::PowderSnow,
        Self::OnTopOfPowderSnow,
        Self::Fence,
        Self::Lava,
        Self::Water,
        Self::WaterBorder,
        Self::Rail,
        Self::UnpassableRail,
        Self::FireInNeighbor,
        Self::Fire,
        Self::DamagingInNeighbor,
        Self::Damaging,
        Self::DoorOpen,
        Self::DoorWoodClosed,
        Self::DoorIronClosed,
        Self::Breach,
        Self::Leaves,
        Self::StickyHoney,
        Self::Cocoa,
        Self::DamageCautious,
        Self::OnTopOfTrapdoor,
        Self::BigMobsCloseToDanger,
    ];
    /// The vanilla `PathType` constant `name` names, as the block data writes
    /// it (`"WATER"`); an unknown name is `Blocked`.
    pub fn from_name(name: &str) -> Self {
        match name {
            "OPEN" => Self::Open,
            "WALKABLE" => Self::Walkable,
            "WALKABLE_DOOR" => Self::WalkableDoor,
            "TRAPDOOR" => Self::Trapdoor,
            "POWDER_SNOW" => Self::PowderSnow,
            "ON_TOP_OF_POWDER_SNOW" => Self::OnTopOfPowderSnow,
            "FENCE" => Self::Fence,
            "LAVA" => Self::Lava,
            "WATER" => Self::Water,
            "WATER_BORDER" => Self::WaterBorder,
            "RAIL" => Self::Rail,
            "UNPASSABLE_RAIL" => Self::UnpassableRail,
            "FIRE_IN_NEIGHBOR" => Self::FireInNeighbor,
            "FIRE" => Self::Fire,
            "DAMAGING_IN_NEIGHBOR" => Self::DamagingInNeighbor,
            "DAMAGING" => Self::Damaging,
            "DOOR_OPEN" => Self::DoorOpen,
            "DOOR_WOOD_CLOSED" => Self::DoorWoodClosed,
            "DOOR_IRON_CLOSED" => Self::DoorIronClosed,
            "BREACH" => Self::Breach,
            "LEAVES" => Self::Leaves,
            "STICKY_HONEY" => Self::StickyHoney,
            "COCOA" => Self::Cocoa,
            "DAMAGE_CAUTIOUS" => Self::DamageCautious,
            "ON_TOP_OF_TRAPDOOR" => Self::OnTopOfTrapdoor,
            "BIG_MOBS_CLOSE_TO_DANGER" => Self::BigMobsCloseToDanger,
            _ => Self::Blocked,
        }
    }
    /// PathType's default malus.
    pub fn malus(self) -> f32 {
        match self {
            Self::Blocked | Self::PowderSnow | Self::Fence | Self::Lava | Self::UnpassableRail => {
                -1.0
            }
            Self::Damaging | Self::DoorWoodClosed | Self::DoorIronClosed | Self::Leaves => -1.0,
            Self::Water
            | Self::WaterBorder
            | Self::FireInNeighbor
            | Self::DamagingInNeighbor
            | Self::StickyHoney => 8.0,
            Self::Fire => 16.0,
            Self::Breach | Self::BigMobsCloseToDanger => 4.0,
            _ => 0.0,
        }
    }
}
/// `Pose`: the stance a living entity's box follows (`Entity.setPose`),
/// vanilla 26.3's set. A species' own box reads its state where its
/// `getDefaultDimensions` does; a stance a species never takes answers its
/// standing box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum Pose {
    Standing,
    FallFlying,
    Sleeping,
    Swimming,
    SpinAttack,
    Crouching,
    LongJumping,
    Dying,
    Croaking,
    UsingTongue,
    Sitting,
    Roaring,
    Sniffing,
    Emerging,
    Digging,
    Sliding,
    Shooting,
    Inhaling,
}
impl Pose {
    /// The pose the network id (`Pose.id`) names; an unknown id is standing
    /// (`ByIdMap`'s out-of-bounds strategy).
    pub fn from_id(id: i32) -> Self {
        match id {
            1 => Self::FallFlying,
            2 => Self::Sleeping,
            3 => Self::Swimming,
            4 => Self::SpinAttack,
            5 => Self::Crouching,
            6 => Self::LongJumping,
            7 => Self::Dying,
            8 => Self::Croaking,
            9 => Self::UsingTongue,
            10 => Self::Sitting,
            11 => Self::Roaring,
            12 => Self::Sniffing,
            13 => Self::Emerging,
            14 => Self::Digging,
            15 => Self::Sliding,
            16 => Self::Shooting,
            17 => Self::Inhaling,
            _ => Self::Standing,
        }
    }
}
