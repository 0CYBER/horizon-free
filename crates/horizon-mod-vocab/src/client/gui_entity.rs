//! A living entity drawn in a screen (`DrawContext::entity`): vanilla's
//! `InventoryScreen.renderEntityInInventoryFollowsMouse` and
//! `renderEntityInInventory`, which recipe viewers' mob pages, Jade's entity
//! icon and Hostile Neural Networks' deep learner draw their mobs with.

use crate::Compound;

/// Which living entity [`DrawContext::entity`](crate::DrawContext::entity) draws.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum GuiEntity {
    /// A new entity of living type `kind` (`minecraft:zombie`, `mymod:golem`), made
    /// for the screen and never added to a level, loaded from `data` as `/summon
    /// <kind> ~ ~ ~ <data>` reads it (`{IsBaby:1b}`, a variant); empty `data` is
    /// the type's defaults. The game keeps the few it made last, so drawing the
    /// same `kind` and `data` every frame makes it once, as a screen that keeps
    /// its `LivingEntity` does.
    Kind {
        /// The living entity type's id.
        kind: String,
        /// The entity's saved fields, without `id` or `Pos`.
        data: Compound,
    },
    /// An entity of the client's level, as it is this frame (its equipment,
    /// its pose), turned for the screen: what `InventoryScreen` draws the
    /// player with. The handle is a client handle (`ClientHost::crosshair_target`).
    Existing(crate::EntityRef),
}

impl GuiEntity {
    /// A new entity of living type `kind` with its defaults.
    pub fn kind(kind: impl Into<String>) -> Self {
        Self::Kind {
            kind: kind.into(),
            data: Compound::new(),
        }
    }
    /// A new entity of living type `kind` loaded from `data`.
    pub fn with_data(kind: impl Into<String>, data: Compound) -> Self {
        Self::Kind {
            kind: kind.into(),
            data,
        }
    }
    /// An entity of the client's level.
    pub fn existing(entity: crate::EntityRef) -> Self {
        Self::Existing(entity)
    }
}

/// How a [`GuiEntity`] is turned.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum GuiEntityLook {
    /// `renderEntityInInventoryFollowsMouse`: the body and head turned toward
    /// this point in GUI pixels (the cursor, as the inventory's player does),
    /// by vanilla's `atan((centre - point) / 40)` angles.
    At([f32; 2]),
    /// `renderEntityInInventory` with a turn of the mod's own: the body `yaw`
    /// degrees from facing the viewer (counterclockwise seen from above; a
    /// spinning mob adds to it each frame), the head pitched `pitch` degrees
    /// (down is positive), the head facing where the body does.
    Turned {
        /// The body's turn, degrees.
        yaw: f32,
        /// The head's pitch, degrees.
        pitch: f32,
    },
}

/// Where and how big a [`GuiEntity`] is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct GuiEntityView {
    /// The box (x0, y0, x1, y1, GUI pixels) the entity stands centred in;
    /// what lies outside it is cut off, as vanilla's picture-in-picture box.
    pub rect: [f32; 4],
    /// GUI pixels per block (vanilla's `size`: 30 for the inventory's player).
    pub size: f32,
    /// Blocks the entity is moved down from the box's centre (vanilla's
    /// `offsetY`: 0.0625 for the inventory's player).
    pub offset_y: f32,
    /// How it is turned.
    pub look: GuiEntityLook,
}

impl GuiEntityView {
    /// A view in the same units as its public fields.
    pub fn new(rect: [f32; 4], size: f32, offset_y: f32, look: GuiEntityLook) -> Self {
        Self {
            rect,
            size,
            offset_y,
            look,
        }
    }
}

/// Why [`DrawContext::entity`](crate::DrawContext::entity) drew nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum GuiEntityError {
    /// No living entity type has that id (unknown, or not a living type).
    UnknownType,
    /// `data` names a different type in its `id`, or doesn't load as that type.
    BadData,
    /// The handle names no living entity of the client's level: stale, gone,
    /// a server handle, or an entity that isn't living.
    NoEntity,
    /// The box is empty, or `size`, `offset_y` or the look isn't finite, or
    /// `size` isn't positive.
    BadView,
    /// The context has no world to draw in (a drawing surface with no game).
    Unavailable,
}

impl std::fmt::Display for GuiEntityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::UnknownType => "no living entity type by that id",
            Self::BadData => "the entity data doesn't load as that type",
            Self::NoEntity => "the handle names no living entity of the client's level",
            Self::BadView => "the view's box, size, offset or look is not drawable",
            Self::Unavailable => "no game to draw the entity in",
        })
    }
}

impl std::error::Error for GuiEntityError {}

impl GuiEntityView {
    /// Whether the view can be drawn: a non-empty box, a positive size, all finite.
    pub fn is_drawable(&self) -> bool {
        let look = match self.look {
            GuiEntityLook::At(p) => p.iter().all(|v| v.is_finite()),
            GuiEntityLook::Turned { yaw, pitch } => yaw.is_finite() && pitch.is_finite(),
        };
        let [x0, y0, x1, y1] = self.rect;
        let finite = [x0, y0, x1, y1, self.size, self.offset_y]
            .iter()
            .all(|v| v.is_finite());
        look && finite && x1 > x0 && y1 > y0 && self.size > 0.0
    }
}
