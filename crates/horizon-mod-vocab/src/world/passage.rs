//! Passages within one level (#3775): an opening a mod's server side registers with
//! `LevelRef::register_passage`, which the engine tells the clients that hold its chunk
//! (docs/modding/src/how-to/passages.md).
use crate::{DimensionKey, Vec3};

/// An opening in a level that leads to another place in the same level, or in another
/// open level with `destination_level`, as Immersive Portals' portal: a rectangle centred on `origin`, `width` blocks along `axis_w` and
/// `height` along `axis_h`, whose centre comes out at `destination`, turned by `turn`.
/// It leads one way: a way back is a second passage. Built with a literal ending in
/// `..Passage::default()`; the default (no size, at the origin) is refused until given a size
/// and axes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
// exhaustive: the opening's rectangle and where it leads; mods build it with a literal
#[derive(horizon_mod_api_macros::Wire)]
pub struct Passage {
    /// The centre of the opening.
    pub origin: Vec3,
    /// The direction of the opening's width: a unit vector.
    pub axis_w: Vec3,
    /// The direction of the opening's height: a unit vector at a right angle to `axis_w`.
    pub axis_h: Vec3,
    /// The opening's size along `axis_w`, in blocks.
    pub width: f64,
    /// The opening's size along `axis_h`, in blocks.
    pub height: f64,
    /// Where `origin` comes out.
    pub destination: Vec3,
    /// How the far side is turned about the vertical axis.
    pub turn: PassageTurn,
    /// The level `destination` is in: `None` for the passage's own level. Another level must
    /// be open when the passage is registered, and a passage into one is not sent to clients
    /// and is refused `PassageCrossRefusal::FarLevel` when crossed, until clients can follow
    /// a player into another level.
    pub destination_level: Option<DimensionKey>,
}

impl Passage {
    /// The chunk the engine keys the passage to (`[x, z]`): the one that holds `origin`.
    /// A client is told of the passage while it holds this chunk, and the per-chunk cap
    /// counts here.
    pub fn chunk(&self) -> [i32; 2] {
        // `as` saturates: a registered origin is inside the world, far within `i32`.
        [self.origin.x, self.origin.z].map(|v| (v.floor() as i32) >> 4)
    }
    /// `v` turned by the passage's turn about the vertical axis.
    pub fn turned(&self, v: Vec3) -> Vec3 {
        match self.turn {
            PassageTurn::None => v,
            PassageTurn::Clockwise90 => Vec3::new(-v.z, v.y, v.x),
            PassageTurn::Clockwise180 => Vec3::new(-v.x, v.y, -v.z),
            PassageTurn::Counterclockwise90 => Vec3::new(v.z, v.y, -v.x),
        }
    }
}

/// A turn about the vertical axis, seen from above, as vanilla's `Rotation`: a clockwise
/// quarter turn takes south (+z) to west (-x).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PassageTurn {
    /// No turn.
    #[default]
    None,
    /// A clockwise quarter turn.
    Clockwise90,
    /// A half turn.
    Clockwise180,
    /// A counterclockwise quarter turn.
    Counterclockwise90,
}

/// A registered passage, from `register_passage`: what `remove_passage` takes, and what a
/// client names when it crosses. Never given again while the game runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
// exhaustive: its field is private, so mods cannot build or destructure it
#[derive(horizon_mod_api_macros::Wire)]
pub struct PassageId {
    pub(crate) id: u64,
}

impl PassageId {
    /// The passage the server numbered `id`. For the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn from_raw(id: u64) -> Self {
        Self { id }
    }
    /// The server's number for the passage. For the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn raw(self) -> u64 {
        self.id
    }
}

/// Why `register_passage` refused a passage; nothing was registered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PassageRegistrationError {
    /// The level is gone or its handle belongs to an earlier open of the world.
    Stale,
    /// Not on a server.
    Unavailable,
    /// A coordinate or a size is NaN or infinite.
    NotFinite,
    /// An axis is not a unit vector, or the two are not at a right angle.
    BadAxes,
    /// The width or the height is not above 0 and at most `max` blocks.
    BadSize {
        /// The largest side, in blocks: 32.
        max: u32,
    },
    /// A corner of the opening or of where it comes out is past the positions vanilla's
    /// server accepts from a client (`ServerGamePacketListenerImpl.clampHorizontal`,
    /// `clampVertical`): 30,000,000 blocks across, 20,000,000 up or down.
    OutsideWorld,
    /// The level already has `limit` passages: 256.
    LevelFull {
        /// Passages per level at most.
        limit: u32,
    },
    /// The passage's chunk ([`Passage::chunk`]) already has `limit` passages: 8.
    ChunkFull {
        /// Passages per chunk at most.
        limit: u32,
    },
    /// Every number was given: an id is never given twice.
    NoIdsLeft,
    /// `destination_level` is not an open level, or is the passage's own level.
    UnknownLevel,
}

impl From<crate::LevelReadError> for PassageRegistrationError {
    fn from(error: crate::LevelReadError) -> Self {
        match error {
            crate::LevelReadError::Stale => Self::Stale,
            crate::LevelReadError::Unavailable => Self::Unavailable,
        }
    }
}

impl crate::NoBackend for PassageRegistrationError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// Why `cross_passage` refused on the client: the player was not moved and nothing was
/// sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PassageCrossError {
    /// Not a client connected to a server.
    Unavailable,
    /// The window has no player.
    NoPlayer,
    /// An earlier crossing still waits for the server's answer.
    Pending,
    /// The player rides something.
    Riding,
    /// The window holds no passage with this number.
    UnknownPassage,
    /// The player's feet are more than `limit` blocks from the nearest point of the opening.
    TooFar {
        /// How far, in blocks: 20, Immersive Portals' limit.
        limit: u32,
    },
    /// A mod's `client_packet_send` cancelled the crossing's payload to the server.
    Cancelled,
}

impl crate::NoBackend for PassageCrossError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// Why the server refused a client's crossing: it put the player back where it has them,
/// with a position packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PassageCrossRefusal {
    /// The player's level has no passage with this number.
    UnknownPassage,
    /// A teleport, or an earlier crossing, waits for the client's answer.
    Pending,
    /// The client has not loaded the level around the player yet.
    NotLoaded,
    /// The player is dead.
    Dead,
    /// The player sleeps.
    Sleeping,
    /// The player rides something.
    Riding,
    /// The move to where the client crossed from failed the server's move checks.
    MoveRefused,
    /// Where the client crossed from is more than `limit` blocks from the nearest point of
    /// the opening.
    TooFar {
        /// How far, in blocks: 20, Immersive Portals' limit.
        limit: u32,
    },
    /// The opening's chunk is not loaded or not tracked for the player.
    NearEndNotTracked,
    /// The chunk the player comes out in is not loaded or not tracked for the player.
    FarEndNotTracked,
    /// A mod's `player_passage_cross` cancelled the crossing.
    Vetoed,
    /// The passage leads into another level, which a crossing cannot reach yet.
    FarLevel,
}
