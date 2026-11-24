//! Owned drawing data for a screen scene, separate from live world entities.

use crate::{BlockState, Identifier, item::containers::ModBlockEntityFields};

/// A mod block entity's type and public fields supplied to a screen's drawing Part.
/// This is not a live entity: internal cache and revision identities never cross the API.
/// With a scene clock and a native scene ticker registered for this type, these values
/// seed the ticking copy. Changes to the inputs or a clock rewind reset that copy.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct SceneBlockEntity {
    /// The immutable type id of the temporary drawing entity.
    pub id: Identifier,
    /// Saved fields, item slots and menu data, in their existing public form.
    pub fields: ModBlockEntityFields,
}

impl SceneBlockEntity {
    /// Describe an entity to draw without retaining a world entity or its cache identity.
    pub fn new(id: Identifier, fields: ModBlockEntityFields) -> Self {
        Self { id, fields }
    }
}

/// One block of a scene a mod draws in a screen: its local position and state, and the
/// optional mod block entity drawn by its Part as in the world.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct SceneBlock {
    /// Position in the scene, independent of the world's blocks.
    pub pos: [i32; 3],
    /// Owned block state, including properties, whose baked model is drawn.
    /// An invalid or unavailable state refuses the entire scene before drawing.
    pub state: BlockState,
    /// Optional owned block entity drawing data.
    pub entity: Option<SceneBlockEntity>,
}

impl SceneBlock {
    /// Describe one scene-local block and optional entity.
    pub fn new(pos: [i32; 3], state: BlockState, entity: Option<SceneBlockEntity>) -> Self {
        Self { pos, state, entity }
    }
}

/// How a scene is seen: its center, vertical rotation and top tilt in degrees, and
/// the number of GUI pixels a block spans.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct SceneView {
    /// Scene point placed at the GUI rectangle's center, in blocks.
    pub center: [f32; 3],
    /// Counterclockwise rotation seen from above, in degrees.
    pub yaw: f32,
    /// Tilt of the scene's top toward the viewer, in degrees.
    pub pitch: f32,
    /// GUI pixels per block; a nonpositive or nonfinite scale draws nothing.
    pub scale: f32,
}

impl SceneView {
    /// Set the scene's camera using the same units as its public fields.
    pub fn new(center: [f32; 3], yaw: f32, pitch: f32, scale: f32) -> Self {
        Self {
            center,
            yaw,
            pitch,
            scale,
        }
    }
}

/// An entity standing in a scene: the same description `DrawContext::entity` draws (a
/// new living entity of a type, with optional saved data, or an entity of the client's
/// level), prepared and drawn by the same path (its type's renderer and the mods'
/// layers, full bright, made for drawing only), placed at `pos` in the scene's own
/// coordinates and turned as the fields say.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct SceneEntity {
    /// Which entity, as `DrawContext::entity` takes it.
    pub entity: crate::GuiEntity,
    /// Its feet, in the scene's blocks (a block at `[0, 0, 0]` spans 0 to 1).
    pub pos: [f64; 3],
    /// Body yaw in degrees, as an entity's: 0 faces south (+z), 90 west.
    pub yaw: f32,
    /// Head yaw in degrees, in the same frame as `yaw`.
    pub head_yaw: f32,
    /// Head pitch in degrees: positive looks down.
    pub head_pitch: f32,
}

impl SceneEntity {
    /// `entity` at `pos`, facing south with its head straight.
    pub fn new(entity: crate::GuiEntity, pos: [f64; 3]) -> Self {
        Self {
            entity,
            pos,
            yaw: 0.0,
            head_yaw: 0.0,
            head_pitch: 0.0,
        }
    }
    /// The same entity turned: body yaw, head yaw and head pitch in degrees.
    pub fn turned(mut self, yaw: f32, head_yaw: f32, head_pitch: f32) -> Self {
        (self.yaw, self.head_yaw, self.head_pitch) = (yaw, head_yaw, head_pitch);
        self
    }
}

/// A particle a mod adds to a scene, as `ClientLevel.addParticle` adds one to a level, at
/// the scene's coordinates. It is added when the scene's clock next advances.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct SceneParticle {
    /// The particle and its parameters, as `add_particle` takes them.
    pub options: crate::ParticleOptions,
    /// Where it starts, in the scene's blocks.
    pub pos: [f64; 3],
    /// Its velocity in blocks a tick (or the provider's own input, as `add_particle`'s).
    pub velocity: [f64; 3],
}

impl SceneParticle {
    /// Particle `options` at `pos` moving at `velocity`.
    pub fn new(options: crate::ParticleOptions, pos: [f64; 3], velocity: [f64; 3]) -> Self {
        Self {
            options,
            pos,
            velocity,
        }
    }
}

/// A scene's own clock, kept under `id` and the calling mod (give each scene its own id).
/// When `tick` advances, the scene runs the ticks in between: the blocks' animate ticks
/// when `animate_blocks` is set, any registered native scene-only block entity tickers,
/// then the particles' ticks. A jump of more than [`SCENE_CATCH_UP`] ticks resets the
/// ticking copies, clears particles and runs only the last [`SCENE_CATCH_UP`] ticks. The
/// same tick again does not run anything. A tick behind the last one, or changed scene
/// inputs, starts the scene over. Each tick's randomness is seeded from `id` and `tick`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct SceneClock {
    /// Which of the calling mod's scenes this is; a screen showing two gives them two ids.
    pub id: u32,
    /// The scene's tick: the mod's own count, 20 a second for a scene that keeps time.
    pub tick: u64,
    /// How far into the next tick this frame is, 0 to 1, for smooth motion.
    pub partial: f32,
    /// Whether the blocks run their animate ticks (`Block.animateTick`: flames,
    /// smoke, drips) as the client runs them for blocks around the player.
    pub animate_blocks: bool,
}

impl SceneClock {
    /// Scene `id` at `tick`, `partial` into the next tick, with the blocks' animate ticks.
    pub fn new(id: u32, tick: u64, partial: f32) -> Self {
        Self {
            id,
            tick,
            partial,
            animate_blocks: true,
        }
    }
    /// The same clock with the blocks' animate ticks on or off.
    pub fn animate_blocks(mut self, on: bool) -> Self {
        self.animate_blocks = on;
        self
    }
}

/// The most ticks one draw runs to catch a scene's clock up; a longer jump starts over.
pub const SCENE_CATCH_UP: u64 = 20;
/// The animate-tick picks a tick makes among a scene's own blocks (vanilla makes 667
/// pairs around the player for a 16-block radius; a scene is a few blocks).
pub const SCENE_ANIMATE_PICKS: u32 = 64;
/// The most blocks one scene takes ([`SceneError::TooManyBlocks`]).
pub const SCENE_MAX_BLOCKS: usize = 4096;
/// The most entities one scene takes ([`SceneError::TooManyEntities`]).
pub const SCENE_MAX_ENTITIES: usize = 16;
/// The most live particles one scene holds, the blocks' own emissions and the mod's
/// together: past it no new particle is added, and more than this many to add at once
/// is [`SceneError::TooManyParticles`].
pub const SCENE_MAX_PARTICLES: usize = 512;
/// The most living scenes that animate in one frame
/// ([`SceneError::TooManyScenes`]).
pub const SCENE_MAX_ANIMATING: usize = 4;
/// The most overlays one scene draw takes ([`SceneOverlayError::TooManyOverlays`]).
pub const SCENE_MAX_OVERLAYS: usize = 4096;

/// How an overlay is occluded by the scene. Neither mode writes depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum SceneDepth {
    /// Visible where it is at or in front of the scene's geometry.
    Tested,
    /// Visible even behind the scene's geometry.
    ThroughWalls,
}

/// A solid annotation in the scene's block coordinates, with a packed ARGB color.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum SceneOverlay {
    /// A line whose thickness is measured in GUI pixels, independent of the view scale.
    Line {
        /// Starting point in scene coordinates.
        from: [f32; 3],
        /// Ending point in scene coordinates.
        to: [f32; 3],
        /// Finite, positive width in GUI pixels.
        width: f32,
        /// Packed ARGB color, including alpha.
        argb: u32,
        /// Occlusion by the scene.
        depth: SceneDepth,
    },
    /// A filled, double-sided quad.
    Quad {
        /// Four corners in order around the quad, in scene coordinates.
        corners: [[f32; 3]; 4],
        /// Packed ARGB color, including alpha.
        argb: u32,
        /// Occlusion by the scene.
        depth: SceneDepth,
    },
}

impl SceneOverlay {
    /// Describes a solid line. A line seen end-on draws nothing.
    pub fn line(from: [f32; 3], to: [f32; 3], width: f32, argb: u32, depth: SceneDepth) -> Self {
        Self::Line {
            from,
            to,
            width,
            argb,
            depth,
        }
    }

    /// Describes a solid quad. Give the corners in order around its perimeter.
    pub fn quad(corners: [[f32; 3]; 4], argb: u32, depth: SceneDepth) -> Self {
        Self::Quad {
            corners,
            argb,
            depth,
        }
    }
}

/// A scene `DrawContext::draw_scene` draws: its blocks, and optionally the entities
/// standing in it, the particles the mod adds and the clock that ticks them and any
/// registered native scene block entity copies. Blocks without a clock do not tick;
/// disabling [`SceneClock::animate_blocks`] only stops the blocks' animate ticks.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct Scene {
    /// The blocks.
    pub blocks: Vec<SceneBlock>,
    /// Entities standing in the scene; they need a clock.
    pub entities: Vec<SceneEntity>,
    /// Particles to add at the clock's next tick; they need a clock.
    pub particles: Vec<SceneParticle>,
    /// The scene's clock, for a scene that lives.
    pub clock: Option<SceneClock>,
}

impl Scene {
    /// A scene of `blocks` alone: no entities, particles or clock.
    pub fn new(blocks: Vec<SceneBlock>) -> Self {
        Self {
            blocks,
            entities: Vec::new(),
            particles: Vec::new(),
            clock: None,
        }
    }
    /// The same scene on `clock`.
    pub fn with_clock(mut self, clock: SceneClock) -> Self {
        self.clock = Some(clock);
        self
    }
    /// The same scene with `entities` standing in it.
    pub fn with_entities(mut self, entities: Vec<SceneEntity>) -> Self {
        self.entities = entities;
        self
    }
    /// The same scene with `particles` to add.
    pub fn with_particles(mut self, particles: Vec<SceneParticle>) -> Self {
        self.particles = particles;
        self
    }
}

/// A rigid pose for a consecutive group in [`Scene::blocks`], without moving
/// the scene camera or rounding its blocks to different cells.
#[derive(Clone, Copy, Debug, PartialEq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct SceneSection {
    /// Index of the first block in the section.
    pub first: u32,
    /// Number of blocks in the section, at least one.
    pub count: u32,
    /// Translation in scene axes, in blocks, applied after rotation.
    pub translation: [f32; 3],
    /// Angles in degrees, applied about the scene's X, Y, then Z axes.
    pub rotation: [f32; 3],
    /// Rotation center in the original scene coordinates, in blocks.
    pub pivot: [f32; 3],
}

impl SceneSection {
    /// Pose `count` blocks starting at `first` around `pivot`, then translate them.
    pub fn new(
        first: u32,
        count: u32,
        translation: [f32; 3],
        rotation: [f32; 3],
        pivot: [f32; 3],
    ) -> Self {
        Self {
            first,
            count,
            translation,
            rotation,
            pivot,
        }
    }
}

/// Why a draw with posed sections was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum SceneSectionError {
    /// The same refusal as an ordinary `DrawContext::draw_scene`.
    Scene(SceneError),
    /// The section had an empty, overlapping, unsorted or out-of-range block
    /// span, or a pose that could not be represented.
    BadSection {
        /// Index in the caller's section list.
        index: u32,
    },
}

impl From<SceneError> for SceneSectionError {
    fn from(error: SceneError) -> Self {
        Self::Scene(error)
    }
}

/// Why `draw_scene` refused a scene. Nothing of a refused scene is drawn, except for
/// [`TooManyScenes`](Self::TooManyScenes), which draws the blocks and the scene's last
/// state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum SceneError {
    /// Not a client's drawing (a server, or no game window).
    Unavailable,
    /// The view or box is not finite, its scale isn't above 0, or the scene seen through
    /// it would not stay in finite GUI coordinates.
    BadView,
    /// More than [`SCENE_MAX_BLOCKS`] blocks.
    TooManyBlocks,
    /// More than [`SCENE_MAX_ENTITIES`] entities.
    TooManyEntities,
    /// More than [`SCENE_MAX_PARTICLES`] particles to add at once.
    TooManyParticles,
    /// [`SCENE_MAX_ANIMATING`] other scenes already animated this frame: this one drew its
    /// blocks and its last state, and its clock did not advance.
    TooManyScenes,
    /// A block state this game doesn't have.
    BadBlock {
        /// The block's index in `blocks`.
        index: u32,
    },
    /// No living entity type has the entity's id, as `DrawContext::entity` refuses it.
    UnknownType {
        /// The entity's index in `entities`.
        index: u32,
    },
    /// The entity's data names another type or doesn't load, as `DrawContext::entity`
    /// refuses it.
    BadData {
        /// The entity's index in `entities`.
        index: u32,
    },
    /// The entity's handle names no living entity of the client's level, as
    /// `DrawContext::entity` refuses it.
    NoEntity {
        /// The entity's index in `entities`.
        index: u32,
    },
    /// An entity's position or turn isn't finite, or lies more than 30 million blocks out.
    BadEntity {
        /// The entity's index in `entities`.
        index: u32,
    },
    /// A particle's options, position or velocity aren't finite, or name a stack.
    BadParticle {
        /// The particle's index in `particles`.
        index: u32,
    },
    /// The clock's `partial` isn't finite.
    BadClock,
    /// Entities or particles without a clock.
    NoClock,
}

/// Why `draw_scene_with_overlays` refused a scene or its overlays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum SceneOverlayError {
    /// The base scene was refused. If the animation budget was exceeded, the scene's
    /// blocks, last state and overlays were still drawn.
    Scene(SceneError),
    /// More than [`SCENE_MAX_OVERLAYS`] overlays.
    TooManyOverlays,
    /// Nonfinite overlay coordinates, nonfinite projected coordinates, or a nonfinite
    /// or nonpositive line width.
    BadOverlay {
        /// The overlay's index in `overlays`.
        index: u32,
    },
}

impl From<SceneError> for SceneOverlayError {
    fn from(error: SceneError) -> Self {
        Self::Scene(error)
    }
}

/// The most textured quads one scene draw takes.
pub const SCENE_MAX_TEXTURED_QUADS: usize = 4096;

/// A two-sided textured face in the scene, drawn at its own depth and clipped with
/// the blocks. Draw with [`DrawContext::draw_scene_with_quads`](crate::DrawContext::draw_scene_with_quads).
#[derive(Clone, Debug, PartialEq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct SceneTexturedQuad {
    /// An existing sprite in the GUI/scene atlas, e.g. `minecraft:block/water_still`
    /// (also `block/water_still`) or `mymod:gui/panel`.
    pub sprite: String,
    /// Scene-local corners in order around the face, in blocks.
    pub corners: [[f32; 3]; 4],
    /// One `[u, v]` per corner, each component in `0..=1` relative to the sprite;
    /// `[0, 0]` is its top left.
    pub uv: [[f32; 2]; 4],
    /// Sprite multiplier, including alpha, in `0xAARRGGBB` (opaque white leaves it unchanged).
    pub tint: u32,
}

impl SceneTexturedQuad {
    /// A face with its four corners and matching sprite coordinates, and an ARGB tint.
    pub fn new(
        sprite: impl Into<String>,
        corners: [[f32; 3]; 4],
        uv: [[f32; 2]; 4],
        tint: u32,
    ) -> Self {
        Self {
            sprite: sprite.into(),
            corners,
            uv,
            tint,
        }
    }
}

/// Why [`DrawContext::draw_scene_with_quads`](crate::DrawContext::draw_scene_with_quads)
/// refused a scene. Invalid quads leave the drawing and scene's clock unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum SceneWithQuadsError {
    /// The underlying scene draw's result. [`SceneError::TooManyScenes`] still draws
    /// the blocks, textured quads and last state without advancing the clock.
    Scene(SceneError),
    /// More than [`SCENE_MAX_TEXTURED_QUADS`] textured quads.
    TooManyTexturedQuads,
    /// One of the quad's coordinates isn't finite or is more than 30 million blocks
    /// from the origin on an axis, or a UV component is not in `0..=1`.
    BadTexturedQuad {
        /// The quad's index in `quads`.
        index: u32,
    },
    /// The quad's sprite isn't in the GUI/scene atlas.
    UnknownSprite {
        /// The quad's index in `quads`.
        index: u32,
    },
}
