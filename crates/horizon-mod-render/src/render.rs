//! Render functions for native mods (docs/modding/src/how-to/native-mods.md): empty functions the
//! renderer calls at fixed places of a frame, which a mod's mixins replace, declared with the
//! hooks in `hooks/`.

use glam::{DVec3, Mat4};
use mod_api::LevelRef;
mod generated {
    include!(concat!(env!("OUT_DIR"), "/native_render_generated.rs"));
}
pub use generated::{
    HandDrawnEvent, RenderNeedsEvent, WorldDrawnEvent, WorldViewDrawnEvent, WorldViewsEvent,
};
pub use wgpu;

/// Whether a mixin build installed a patched copy of the free function `$function` of this
/// module. The prebuilt game's staging pass answers it from the function's replacement slot; a
/// build without replacement slots has no patched functions.
macro_rules! replaced {
    ($function:ident) => {
        false
    };
}

/// Whether a mixin replaced the raw world draw function.
pub fn world_drawn_hooked() -> bool {
    replaced!(world_drawn)
}

/// What a mod can draw with once the world is drawn: after the sky, terrain, entities,
/// particles, weather and the world post pass, before the first-person hand ([`HandDrawn`]),
/// the HUD and the screens. `color` holds the world's picture (copy it out before reading it: it is the target
/// drawn into), `depth` the world's depth, reversed (0 is the sky).
#[non_exhaustive]
pub struct WorldDrawn<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The frame's target and a view of it in `color_format`.
    pub color: &'a wgpu::Texture,
    pub color_view: &'a wgpu::TextureView,
    pub color_format: wgpu::TextureFormat,
    /// The size of `color`.
    pub size: [u32; 2],
    /// The world's depth (Depth32Float, sampleable), and its size: smaller than `size` when the
    /// world draws at a lower resolution. It holds the frame's depth only when a mod asked for
    /// it ([`RenderNeeds::world_depth`]) or something else kept it.
    pub depth: &'a wgpu::TextureView,
    pub depth_size: [u32; 2],
    /// Camera-relative world to clip space, without TAA's jitter, and with it (what the world
    /// was drawn with).
    pub view_proj: Mat4,
    pub jittered: Mat4,
    /// The camera's position in the world.
    pub camera: DVec3,
    /// The camera-relative entity geometry of the frame (`Frame::entities`), quads of 4
    /// vertices, and its runs by entity when a mod asked for them
    /// ([`RenderNeeds::entity_spans`]) or a shader pack draws them.
    pub entities: &'a [Vertex],
    pub spans: &'a [EntitySpan],
    /// Which entity each run of `entities` is, recorded every frame (`Frame::entity_motion`):
    /// the run's first vertex and the entity, `(1 << 32) | id` for a mob or a player and
    /// `(2 << 32) | id` for a dropped item (their entity ids), 0 for anything else. A run lasts
    /// until the next. Unlike `spans`, whose ids name a type, these follow one entity from
    /// frame to frame.
    pub motion: &'a [(u32, u64)],
    /// The shared atlas entity UVs point into.
    pub atlas: Option<&'a wgpu::Texture>,
    /// Quad indices (0 1 2 0 2 3 for each 4 vertices), for as many quads as `entities` holds.
    pub quad_indices: &'a wgpu::Buffer,
    /// The block textures the terrain samples, for drawing blocks with the player's own
    /// textures (`block_sprites` says where a block's are).
    pub block_sprites: BlockSpriteArrays<'a>,
}

/// The block sprite arrays ([`WorldDrawn::block_sprites`]): each texture a whole square layer,
/// its UVs 0 to 1 across it, as the world uses them (gamma-encoded, animated layers at the frame
/// shown).
#[non_exhaustive]
pub struct BlockSpriteArrays<'a> {
    /// The arrays as `D2Array` views, terrain.wgsl's eight (`sprites`, `sprites_1`...): a 1-layer
    /// stand-in past the ones in use, each with the mip levels the Mipmap Levels option leaves.
    pub views: &'a [wgpu::TextureView],
    /// Layers per array on this device (2048 on most; fewer where the GPU holds fewer, and
    /// OpenGL pads each array past it): a sprite's `array` and `layer` are cut by it.
    pub split: u32,
    /// A layer's width and height in texels.
    pub size: u32,
    /// Nearest, repeating, the nearest mip level: texels drawn sharp.
    pub nearest: &'a wgpu::Sampler,
    /// The terrain's own (linear, clamped, anisotropic under that Texture Filtering).
    pub filtered: &'a wgpu::Sampler,
}

/// What a mod can draw with once the first-person hand is drawn ([`hand_drawn`]): after
/// the world and the hand, before entity outlines, post effects, screen effects and the HUD.
/// `color` holds the frame's picture with the hand in it (copy it out before reading it).
#[non_exhaustive]
pub struct HandDrawn<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The frame's target and a view of it in `color_format`.
    pub color: &'a wgpu::Texture,
    pub color_view: &'a wgpu::TextureView,
    pub color_format: wgpu::TextureFormat,
    /// The size of `color`.
    pub size: [u32; 2],
    /// The hand and held item just drawn, camera-relative quads of 4 vertices: empty when no
    /// hand drew (third person, F1, a sleeping or spectating player) or a shader pack drew it.
    pub hand: &'a [Vertex],
    /// Camera-relative world to clip space for the hand: its own field of view
    /// (`hand_fov_degrees`) and the view bob, without TAA's jitter (the hand has none). Its
    /// depth is the hand's own: the hand's pass clears depth and keeps none.
    pub view_proj: Mat4,
    /// The camera's position in the world.
    pub camera: DVec3,
    /// The shared atlas the hand's UVs point into.
    pub atlas: Option<&'a wgpu::Texture>,
    /// Quad indices (0 1 2 0 2 3 for each 4 vertices).
    pub quad_indices: &'a wgpu::Buffer,
}

/// What the mods need from a frame that the game doesn't make for itself. Each mod sets what it
/// needs and leaves the rest, so several can ask.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RenderNeeds {
    /// Record `Frame::entity_spans` without a shader pack: entity runs get a stable id from
    /// their type's name, block entity runs their block state.
    pub entity_spans: bool,
    /// Keep the world pass's depth for [`world_drawn`] when nothing else does.
    pub world_depth: bool,
}

/// The world is drawn (see [`WorldDrawn`]). Not called on the title screen's panorama.
#[inline(never)]
#[horizon_mod_api_macros::replaceable]
pub fn world_drawn(world: &mut WorldDrawn<'_>) {
    let _ = world;
}

/// The first-person hand is drawn (see [`HandDrawn`]), on every frame [`world_drawn`]
/// runs, with no hand too.
#[inline(never)]
#[horizon_mod_api_macros::replaceable]
pub fn hand_drawn(hand: &mut HandDrawn<'_>) {
    let _ = hand;
}

/// Asked as a frame is built and drawn: set what your mod needs.
#[inline(never)]
#[horizon_mod_api_macros::replaceable]
pub fn render_needs(needs: &mut RenderNeeds) {
    let _ = needs;
}

/// What the mods need this frame ([`render_needs`]).
pub fn needs() -> RenderNeeds {
    let mut needs = RenderNeeds::default();
    render_needs(&mut needs);
    needs
}

/// The limits of [`world_views`] (the engine's own constants) and why a view is refused.
pub use horizon_client_render::render_views::{
    MAX_WORLD_VIEW_DEPTH, MAX_WORLD_VIEW_SIDE, MAX_WORLD_VIEWS, WORLD_VIEW_PIXEL_BUDGET,
    WorldViewRefusal, WorldViewRefusalReason,
};

/// A view of the world a mod asks for ([`world_views`]): the same world, drawn again from a
/// camera of its own into a target of its own (a camera monitor, a second camera's preview, a
/// still for a film strip).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WorldView {
    /// The mod's name for the view: [`world_view_drawn`] hands its picture back with it, and
    /// the target stays the view's while each frame asks for it again.
    pub id: u64,
    /// Where the camera is in the world.
    pub camera: DVec3,
    /// Degrees, as the player's camera turns: yaw, pitch, and a roll about the view direction
    /// (positive turns the picture counterclockwise).
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    /// The vertical field of view, in degrees (1 to 170).
    pub fov_degrees: f32,
    /// The target's width and height, in pixels (1 to [`MAX_WORLD_VIEW_SIDE`]).
    pub size: [u32; 2],
    /// The plane behind which nothing is drawn ([`WorldView::with_clip_plane`]).
    pub clip_plane: Option<[f64; 4]>,
    /// The picture is mirrored left to right ([`WorldView::with_mirror`]).
    pub mirror: bool,
    /// The view this one is seen in ([`WorldView::with_parent`]).
    pub parent: Option<u64>,
    /// The level the view draws ([`WorldView::with_level`]); `None` is the player's own.
    pub level: Option<LevelRef>,
}

impl WorldView {
    pub fn new(
        id: u64,
        camera: DVec3,
        yaw: f32,
        pitch: f32,
        fov_degrees: f32,
        size: [u32; 2],
    ) -> Self {
        Self {
            id,
            camera,
            yaw,
            pitch,
            roll: 0.0,
            fov_degrees,
            size,
            clip_plane: None,
            mirror: false,
            parent: None,
            level: None,
        }
    }

    pub fn with_roll(self, roll: f32) -> Self {
        Self { roll, ..self }
    }

    /// Draws only what lies in front of a world-space plane `[x, y, z, d]`: the points `p` with
    /// `x * p.x + y * p.y + z * p.z + d >= 0`. Terrain, entities and block entities behind it are
    /// left out, as a wall behind a portal's far side is. The camera must be behind the plane
    /// (where `x * camera.x + y * camera.y + z * camera.z + d < 0`): the plane becomes the
    /// view's near plane, which every backend draws. A view whose camera is not behind it, or
    /// whose plane has a normal `[x, y, z]` of zero, a value that is not finite, or a scaled form
    /// or distance from the camera that is not (as f32 too), is out of range and left out.
    pub fn with_clip_plane(self, plane: [f64; 4]) -> Self {
        Self {
            clip_plane: Some(plane),
            ..self
        }
    }

    /// The camera's picture mirrored left to right, as a mirror shows it: give the camera
    /// reflected through the mirror. A mirrored picture turns every triangle's winding round,
    /// so the view draws with face culling flipped to make up for it: the faces culled are the
    /// ones an unmirrored view culls, and back faces stay hidden.
    pub fn with_mirror(self) -> Self {
        Self {
            mirror: true,
            ..self
        }
    }

    /// Seen in the view `parent` (a portal's far side seen through another portal): push it
    /// after its parent. It is drawn before its parent, so that in the parent's
    /// [`world_view_drawn`] its `color` and `depth` hold this frame's picture, to draw into the
    /// parent's target there. A view with no parent is at depth 1, one seen in it at 2; up to
    /// [`MAX_WORLD_VIEW_DEPTH`].
    pub fn with_parent(self, parent: u64) -> Self {
        Self {
            parent: Some(parent),
            ..self
        }
    }

    /// Draws the level `level`, a handle from `host.level(dimension)`: `None` is the player's
    /// own level, and so is a handle naming it. Viewed levels are not drawn yet: a view of any
    /// other level, an open viewed one, one whose handle is stale (its epoch changed) or one
    /// closed or never open, is refused as [`WorldViewRefusalReason::Level`]. The handle is
    /// resolved once a frame, so a handle that goes stale is refused from the next frame on.
    pub fn with_level(self, level: Option<LevelRef>) -> Self {
        Self { level, ..self }
    }
}

/// The views asked for this frame ([`world_views`]): push each one.
pub struct WorldViews<'a> {
    views: &'a mut horizon_client_render::render_views::WorldViews,
}

impl WorldViews<'_> {
    /// Asks for `view` this frame, or refuses it, naming it and why
    /// ([`WorldViewRefusalReason`]): a value out of range, an id already pushed this frame, a
    /// parent not pushed before it, deeper than [`MAX_WORLD_VIEW_DEPTH`], past
    /// [`MAX_WORLD_VIEWS`] views, or past [`WORLD_VIEW_PIXEL_BUDGET`] pixels with the views
    /// pushed before it. Each refusal is logged once.
    pub fn push(&mut self, view: WorldView) -> Result<(), WorldViewRefusal> {
        let mut engine = horizon_client_render::render_views::WorldView::new(
            view.id,
            view.camera,
            view.yaw,
            view.pitch,
            view.fov_degrees,
            view.size,
        )
        .with_roll(view.roll);
        engine.clip_plane = view.clip_plane;
        engine.mirror = view.mirror;
        engine.parent = view.parent;
        engine.level = view.level;
        self.views.push(engine)
    }

    /// How many views were taken this frame so far.
    pub fn len(&self) -> usize {
        self.views.views().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A world view just drawn ([`world_view_drawn`]): its picture and depth, and what a mod needs
/// to draw into it or copy it out. Drawn after the frame's world and before [`world_drawn`], so
/// a mod can show it in the frame's picture there.
#[non_exhaustive]
pub struct WorldViewDrawn<'a> {
    /// The [`WorldView::id`] it was asked with.
    pub id: u64,
    /// The view it is seen in ([`WorldView::with_parent`]), drawn after it.
    pub parent: Option<u64>,
    /// How deep it is seen in views: 1 for a view with no parent, its parent's and 1 for one
    /// with ([`MAX_WORLD_VIEW_DEPTH`]). (`depth` is its depth texture.)
    pub nesting_depth: u32,
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The view's picture (the world's light in `color_format`, as [`WorldDrawn::color`] holds
    /// it, before the tone curve and without the hand or the HUD), and a view of it. The target
    /// is the view's while it is asked for each frame and is drawn over the next frame: copy it
    /// (it can be copied and sampled) to keep a still.
    pub color: &'a wgpu::Texture,
    pub color_view: &'a wgpu::TextureView,
    pub color_format: wgpu::TextureFormat,
    pub size: [u32; 2],
    /// Its depth (Depth32Float, reversed: 0 is the sky), sampleable.
    pub depth: &'a wgpu::TextureView,
    /// World relative to [`WorldViewDrawn::camera`] to clip space, as the picture and `depth`
    /// were drawn: its near plane is the view's clip plane when it has one, and its x is
    /// flipped when the view is mirrored.
    pub view_proj: Mat4,
    /// The view's camera.
    pub camera: DVec3,
}

/// Asked once as each frame is built (not on the title screen's panorama): push the world views
/// your mod wants drawn this frame, each parent before the views seen in it.
/// [`WorldViews::push`] takes each or refuses it by name ([`WorldViewRefusal`]), and logs each
/// refusal once. Keep it cheap.
#[inline(never)]
#[horizon_mod_api_macros::replaceable]
pub fn world_views(views: &mut WorldViews<'_>) {
    let _ = views;
}

/// A world view is drawn (see [`WorldViewDrawn`]): once per view taken this frame, the views
/// seen in a view before it.
#[inline(never)]
#[horizon_mod_api_macros::replaceable]
pub fn world_view_drawn(view: &mut WorldViewDrawn<'_>) {
    let _ = view;
}

/// Whether a mixin replaced the world views function: while none has, the renderer does not
/// ask.
pub fn world_views_hooked() -> bool {
    replaced!(world_views)
}

pub use horizon_client_render::render_geometry::{EntitySpan, Vertex};

/// Install the native render adapters before engine startup.
pub fn install() {
    use horizon_client_render::render_slots as engine;
    engine::install(engine::Slots {
        world_drawn: |v| {
            world_drawn(&mut WorldDrawn {
                device: v.device,
                queue: v.queue,
                encoder: &mut *v.encoder,
                color: v.color,
                color_view: v.color_view,
                color_format: v.color_format,
                size: v.size,
                depth: v.depth,
                depth_size: v.depth_size,
                view_proj: v.view_proj,
                jittered: v.jittered,
                camera: v.camera,
                entities: v.entities,
                spans: v.spans,
                motion: v.motion,
                atlas: v.atlas,
                quad_indices: v.quad_indices,
                block_sprites: BlockSpriteArrays {
                    views: v.block_sprites.views,
                    split: v.block_sprites.split,
                    size: v.block_sprites.size,
                    nearest: v.block_sprites.nearest,
                    filtered: v.block_sprites.filtered,
                },
            })
        },
        hand_drawn: |v| {
            hand_drawn(&mut HandDrawn {
                device: v.device,
                queue: v.queue,
                encoder: &mut *v.encoder,
                color: v.color,
                color_view: v.color_view,
                color_format: v.color_format,
                size: v.size,
                hand: v.hand,
                view_proj: v.view_proj,
                camera: v.camera,
                atlas: v.atlas,
                quad_indices: v.quad_indices,
            })
        },
        render_needs: |v| {
            let mut needs = RenderNeeds {
                entity_spans: v.entity_spans,
                world_depth: v.world_depth,
            };
            render_needs(&mut needs);
            v.entity_spans = needs.entity_spans;
            v.world_depth = needs.world_depth;
        },
        world_views: |views| world_views(&mut WorldViews { views }),
        world_views_hooked,
        world_view_drawn: |v| {
            world_view_drawn(&mut WorldViewDrawn {
                id: v.id,
                parent: v.parent,
                nesting_depth: v.nesting_depth,
                device: v.device,
                queue: v.queue,
                encoder: &mut *v.encoder,
                color: v.color,
                color_view: v.color_view,
                color_format: v.color_format,
                size: v.size,
                depth: v.depth,
                view_proj: v.view_proj,
                camera: v.camera,
            })
        },
        world_drawn_hooked,
    });
}
