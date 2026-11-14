use glam::{DVec3, Mat4, Vec4};
use mod_api::LevelRef;
use std::fmt;

/// The most world views ([`crate::render_slots::world_views`]) one frame draws.
pub const MAX_WORLD_VIEWS: usize = 8;
/// The largest side of a world view's target, in pixels.
pub const MAX_WORLD_VIEW_SIDE: u32 = 4096;
/// How deep views go in views ([`WorldView::with_parent`]): a view with no parent is at depth 1,
/// one seen in it at 2. Immersive Portals' default `maxPortalLayer`.
pub const MAX_WORLD_VIEW_DEPTH: u32 = 5;
/// The pixels a frame's views take together: four views of [`MAX_WORLD_VIEW_SIDE`] a side, as
/// much as the four views a frame drew before views had parents.
pub const WORLD_VIEW_PIXEL_BUDGET: u64 =
    4 * MAX_WORLD_VIEW_SIDE as u64 * MAX_WORLD_VIEW_SIDE as u64;

/// A view of the world a mod asks for ([`crate::render_slots::world_views`]): the same world, drawn again from a
/// camera of its own into a target of its own (a camera monitor, a second camera's preview, a
/// still for a film strip).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WorldView {
    /// The mod's name for the view: [`crate::render_slots::world_view_drawn`] hands its picture back with it, and
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

    /// Seen in the view `parent`, a portal's far side seen through another portal: pushed after
    /// it ([`WorldViews::push`]), one deeper, and drawn before it, so that in the parent's
    /// [`crate::render_slots::world_view_drawn`] this view's picture is this frame's.
    pub fn with_parent(self, parent: u64) -> Self {
        Self {
            parent: Some(parent),
            ..self
        }
    }

    /// Draws the level `level`, a handle from `host.level(dimension)`: `None` is the player's own
    /// level, and so is a handle naming it. Viewed levels are not drawn yet: a view of any other
    /// level, an open viewed one, one whose handle is stale (its epoch changed) or one closed or
    /// never open, is refused as [`WorldViewRefusalReason::Level`]. The handle is resolved once a
    /// frame, so a handle that goes stale is refused from the next frame on.
    pub fn with_level(self, level: Option<LevelRef>) -> Self {
        Self { level, ..self }
    }

    /// Why its values cannot be drawn on a device whose textures are at most `max_side` pixels
    /// a side, if they cannot.
    fn out_of_range(&self, max_side: u32) -> Option<WorldViewRefusalReason> {
        let finite = self.camera.is_finite()
            && [self.yaw, self.pitch, self.roll]
                .iter()
                .all(|v| v.is_finite());
        let in_side = |limit: u32| self.size.iter().all(|&s| (1..=limit).contains(&s));
        if !finite {
            Some(WorldViewRefusalReason::NotFinite)
        } else if !(1.0..=170.0).contains(&self.fov_degrees) {
            Some(WorldViewRefusalReason::FieldOfView(self.fov_degrees))
        } else if !in_side(MAX_WORLD_VIEW_SIDE) {
            Some(WorldViewRefusalReason::Side(self.size))
        } else if !in_side(max_side) {
            Some(WorldViewRefusalReason::DeviceLimit {
                size: self.size,
                max_side,
            })
        } else {
            match self.clip_plane {
                Some(plane) if self.camera_clip_plane().is_none() => {
                    Some(WorldViewRefusalReason::ClipPlane(plane))
                }
                _ => None,
            }
        }
    }

    /// The clip plane relative to the camera, as the projection takes it: a unit normal and the
    /// camera's distance in front of the plane (below 0, behind it), worked out in f64 so far
    /// from the origin it loses nothing. `None` without one, or for one out of range: a normal
    /// of zero, a value or its scaled or camera-relative form not finite (in f32 too), or the
    /// camera not behind it.
    pub(crate) fn camera_clip_plane(&self) -> Option<Vec4> {
        let [x, y, z, d] = self.clip_plane?;
        let length = DVec3::new(x, y, z).length();
        let normal = DVec3::new(x, y, z) / length;
        let plane = normal
            .as_vec3()
            .extend((normal.dot(self.camera) + d / length) as f32);
        (length > 0.0 && length.is_finite() && plane.is_finite() && plane.w < 0.0).then_some(plane)
    }
}

/// A world view just drawn ([`crate::render_slots::world_view_drawn`]): its picture and depth, and what a mod needs
/// to draw into it or copy it out. Drawn after the frame's world and before [`crate::render_slots::world_drawn`], so
/// a mod can show it in the frame's picture there.
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
    /// The view's picture (the world's light in `color_format`, as [`crate::render_slots::WorldDrawn::color`] holds
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

/// Why a view pushed to [`WorldViews`] was refused, naming the view. Each is logged the first
/// time a view of that `id` is refused for that reason.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WorldViewRefusal {
    /// The refused view's [`WorldView::id`].
    pub id: u64,
    pub reason: WorldViewRefusalReason,
}

/// What of a view, or of the frame, refused it ([`WorldViewRefusal`]).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum WorldViewRefusalReason {
    /// Its camera, yaw, pitch or roll is not finite.
    NotFinite,
    /// Its field of view, outside 1 to 170 degrees.
    FieldOfView(f32),
    /// Its size, a side of 0 or over [`MAX_WORLD_VIEW_SIDE`].
    Side([u32; 2]),
    /// Its size, a side over `max_side`, the largest texture side of this device.
    DeviceLimit { size: [u32; 2], max_side: u32 },
    /// Its clip plane, out of range ([`WorldView::with_clip_plane`]).
    ClipPlane([f64; 4]),
    /// A view pushed before it this frame has its id.
    DuplicateId,
    /// Its parent, not pushed before it this frame (or refused).
    MissingParent(u64),
    /// It would be deeper than [`MAX_WORLD_VIEW_DEPTH`].
    TooDeep,
    /// [`MAX_WORLD_VIEWS`] views were pushed before it this frame.
    TooMany,
    /// Its `pixels` would take this frame's views past [`WORLD_VIEW_PIXEL_BUDGET`], with `left`
    /// to spare.
    PixelBudget { pixels: u64, left: u64 },
    /// The frame turned out to be the title screen's or a loading screen's, which draw no view:
    /// refused after its push, which could not know.
    TitleScreen,
    /// Its level ([`WorldView::with_level`]) is not the player's own: viewed levels are not drawn
    /// yet, and a stale, closed or never open one is refused the same way.
    Level,
}

impl fmt::Display for WorldViewRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use WorldViewRefusalReason as R;
        write!(f, "world view {} refused: ", self.id)?;
        match self.reason {
            R::NotFinite => write!(f, "its camera, yaw, pitch or roll is not finite"),
            R::FieldOfView(fov) => write!(f, "field of view {fov} is outside 1 to 170 degrees"),
            R::Side([w, h]) => write!(f, "size {w}x{h} is outside 1 to {MAX_WORLD_VIEW_SIDE}"),
            R::DeviceLimit {
                size: [w, h],
                max_side,
            } => write!(f, "size {w}x{h} is over this device's {max_side}"),
            R::ClipPlane(p) => write!(f, "clip plane {p:?} is out of range"),
            R::DuplicateId => write!(f, "a view of that id was already pushed this frame"),
            R::MissingParent(parent) => {
                write!(f, "its parent {parent} was not pushed before it this frame")
            }
            R::TooDeep => write!(f, "it would be deeper than {MAX_WORLD_VIEW_DEPTH}"),
            R::TooMany => write!(f, "{MAX_WORLD_VIEWS} views were already pushed this frame"),
            R::PixelBudget { pixels, left } => write!(
                f,
                "its {pixels} pixels are over the {left} left of {WORLD_VIEW_PIXEL_BUDGET}"
            ),
            R::TitleScreen => write!(f, "the title or a loading screen draws no view"),
            R::Level => write!(
                f,
                "its level is not the player's own, which is all that is drawn"
            ),
        }
    }
}

/// The refusals logged so far, each a view's id and its reason's kind: a view refused the same
/// way every frame is logged once. Past `LOGGED` of them, no more are kept or logged.
#[derive(Clone, Copy)]
struct Logged {
    seen: [Option<(u64, std::mem::Discriminant<WorldViewRefusalReason>)>; LOGGED],
    len: usize,
}
const LOGGED: usize = 64;

impl Default for Logged {
    fn default() -> Self {
        Self {
            seen: [None; LOGGED],
            len: 0,
        }
    }
}

impl Logged {
    /// Logs `refusal` unless one like it was.
    fn log(&mut self, refusal: WorldViewRefusal) {
        let key = Some((refusal.id, std::mem::discriminant(&refusal.reason)));
        if self.seen[..self.len].contains(&key) || self.len == LOGGED {
            return;
        }
        self.seen[self.len] = key;
        self.len += 1;
        log::warn!("{refusal}");
        if self.len == LOGGED {
            log::warn!("further world view refusals are not logged");
        }
    }
}

/// The views asked for this frame ([`crate::render_slots::world_views`]): each [`WorldViews::push`]
/// takes a view or refuses it by name. A parent is pushed before the views seen in it, so a
/// cycle cannot be asked for. The list keeps its room from frame to frame.
#[derive(Default)]
pub struct WorldViews {
    views: Vec<WorldView>,
    /// Each view's depth, as `views`.
    depths: [u32; MAX_WORLD_VIEWS],
    /// The pixels `views` take together.
    pixels: u64,
    /// The device's largest texture side (`max_texture_dimension_2d`).
    max_side: u32,
    /// The player's own level this frame, which the game resolved before the views were asked
    /// for (its key and the client level's epoch); `None` before login.
    own_level: Option<LevelRef>,
    logged: Logged,
}

impl WorldViews {
    /// None yet, for a device whose textures are at most `max_side` pixels a side.
    pub fn new(max_side: u32) -> Self {
        Self {
            max_side,
            ..Self::default()
        }
    }

    /// Empties the list for a new frame on a device of `max_side`, keeping what was logged.
    pub fn begin(&mut self, max_side: u32) {
        self.views.clear();
        self.pixels = 0;
        self.max_side = max_side;
    }

    /// The player's own level this frame ([`WorldView::with_level`]): a view naming it is drawn
    /// as one naming none, and one naming any other level is refused. Kept across `begin`.
    pub fn set_own_level(&mut self, own_level: Option<LevelRef>) {
        self.own_level = own_level;
    }

    /// Asks for `view` this frame, or refuses it, naming it and why: a value out of range, an
    /// id already pushed, a parent not pushed before it, a depth past [`MAX_WORLD_VIEW_DEPTH`],
    /// past [`MAX_WORLD_VIEWS`] views, past [`WORLD_VIEW_PIXEL_BUDGET`] pixels or a level other
    /// than the player's own.
    pub fn push(&mut self, view: WorldView) -> Result<(), WorldViewRefusal> {
        match self.depth_for(&view) {
            Ok(depth) => {
                self.depths[self.views.len()] = depth;
                self.pixels += pixels(&view);
                self.views.push(view);
                Ok(())
            }
            Err(reason) => {
                let refusal = WorldViewRefusal {
                    id: view.id,
                    reason,
                };
                self.logged.log(refusal);
                Err(refusal)
            }
        }
    }

    /// `view`'s depth were it pushed now, or why it cannot be.
    fn depth_for(&self, view: &WorldView) -> Result<u32, WorldViewRefusalReason> {
        use WorldViewRefusalReason as R;
        if let Some(reason) = view.out_of_range(self.max_side) {
            return Err(reason);
        }
        if view.level.is_some_and(|l| Some(l) != self.own_level) {
            return Err(R::Level);
        }
        if self.views.iter().any(|v| v.id == view.id) {
            return Err(R::DuplicateId);
        }
        let depth = match view.parent {
            None => 1,
            Some(parent) => match self.views.iter().position(|v| v.id == parent) {
                Some(k) => self.depths[k] + 1,
                None => return Err(R::MissingParent(parent)),
            },
        };
        let left = WORLD_VIEW_PIXEL_BUDGET - self.pixels;
        if depth > MAX_WORLD_VIEW_DEPTH {
            Err(R::TooDeep)
        } else if self.views.len() == MAX_WORLD_VIEWS {
            Err(R::TooMany)
        } else if pixels(view) > left {
            Err(R::PixelBudget {
                pixels: pixels(view),
                left,
            })
        } else {
            Ok(depth)
        }
    }

    /// The views pushed and taken: in the order pushed, then once [`WorldViews::order`] has
    /// run, the order drawn.
    pub fn views(&self) -> &[WorldView] {
        &self.views
    }

    /// The depth of `views()[k]`.
    pub fn depth(&self, k: usize) -> u32 {
        self.depths[k]
    }

    /// Puts the views in the order they draw: deepest first, and in the order pushed within a
    /// depth, so every view draws after the views seen in it. In place, without allocating.
    pub fn order(&mut self) {
        for i in 1..self.views.len() {
            let mut k = i;
            while k > 0 && self.depths[k - 1] < self.depths[k] {
                self.views.swap(k - 1, k);
                self.depths.swap(k - 1, k);
                k -= 1;
            }
        }
    }

    /// Refuses every view taken (the frame is the title screen's), logging each.
    pub fn refuse_all(&mut self, reason: WorldViewRefusalReason) {
        for view in self.views.drain(..) {
            self.logged.log(WorldViewRefusal {
                id: view.id,
                reason,
            });
        }
        self.pixels = 0;
    }

    /// How many refusals were logged so far.
    #[cfg(test)]
    fn logged(&self) -> usize {
        self.logged.len
    }
}

/// A view's target's pixels.
fn pixels(view: &WorldView) -> u64 {
    u64::from(view.size[0]) * u64::from(view.size[1])
}

#[cfg(test)]
mod tests {
    use super::*;
    use WorldViewRefusalReason as R;
    use mod_api::{DimensionKey, WorldEpoch};

    fn view(id: u64, fov: f32, size: [u32; 2]) -> WorldView {
        WorldView::new(id, DVec3::new(1.0, 70.0, -3.0), 30.0, 10.0, fov, size)
    }
    fn refused(id: u64, reason: WorldViewRefusalReason) -> Result<(), WorldViewRefusal> {
        Err(WorldViewRefusal { id, reason })
    }
    fn ids(views: &WorldViews) -> Vec<u64> {
        views.views().iter().map(|v| v.id).collect()
    }
    /// `pushed` refused `id` for the clip plane `plane`, bit for bit (NaN is never equal).
    fn assert_clip_plane_refused(pushed: Result<(), WorldViewRefusal>, id: u64, plane: [f64; 4]) {
        let bits = |p: [f64; 4]| p.map(f64::to_bits);
        assert!(
            matches!(pushed, Err(WorldViewRefusal { id: i, reason: R::ClipPlane(p) })
                if i == id && bits(p) == bits(plane)),
            "{id}: {pushed:?}"
        );
    }

    /// The seven cases left out without a word before (#3774) and a clip plane out of range
    /// are each refused by name, and the view taken stays the only one.
    #[test]
    fn views_out_of_range_are_refused_by_name() {
        let mut views = WorldViews::new(MAX_WORLD_VIEW_SIDE);
        assert_eq!(views.push(view(1, 70.0, [320, 180])), Ok(()));
        assert_eq!(
            views.push(view(1, 70.0, [64, 64])),
            refused(1, R::DuplicateId)
        );
        assert_eq!(
            views.push(view(2, 0.5, [64, 64])),
            refused(2, R::FieldOfView(0.5))
        );
        assert_eq!(
            views.push(view(3, 70.0, [0, 64])),
            refused(3, R::Side([0, 64]))
        );
        assert_eq!(
            views.push(view(4, 70.0, [4097, 64])),
            refused(4, R::Side([4097, 64]))
        );
        assert_eq!(
            views.push(view(5, 70.0, [64, 64]).with_roll(f32::NAN)),
            refused(5, R::NotFinite)
        );
        let far = WorldView::new(
            6,
            DVec3::new(f64::INFINITY, 0.0, 0.0),
            0.0,
            0.0,
            70.0,
            [8, 8],
        );
        assert_eq!(views.push(far), refused(6, R::NotFinite));
        // clip planes: a zero normal, values not finite, the camera (at z -3) in front of the
        // plane or on it, finite values whose scaled or camera-relative form is not
        for (id, plane) in [
            (30, [0.0, 0.0, 0.0, 1.0]),
            (31, [0.0, f64::NAN, 1.0, 0.0]),
            (32, [0.0, 0.0, 1.0, f64::INFINITY]),
            (33, [0.0, 0.0, -1.0, 0.0]),
            (34, [0.0, 0.0, 2.0, 6.0]),
        ] {
            let clipped = view(id, 70.0, [64, 64]).with_clip_plane(plane);
            assert_clip_plane_refused(views.push(clipped), id, plane);
        }
        let at_origin = |id| WorldView::new(id, DVec3::ZERO, 0.0, 0.0, 70.0, [64, 64]);
        for (id, plane) in [
            (35, [0.0, 0.0, 1.0, -1e40]),
            (36, [1e-160, 0.0, 0.0, -1e308]),
        ] {
            let clipped = at_origin(id).with_clip_plane(plane);
            assert_clip_plane_refused(views.push(clipped), id, plane);
        }
        // the camera behind the plane z = 0, mirrored too
        let mirror = view(8, 70.0, [64, 64])
            .with_clip_plane([0.0, 0.0, 0.5, 0.0])
            .with_mirror();
        assert_eq!(views.push(mirror), Ok(()));
        assert_eq!(ids(&views), [1, 8]);
        // A device with smaller textures (downlevel, GL: 2048) refuses what it cannot hold.
        let mut small = WorldViews::new(2048);
        assert_eq!(
            small.push(view(20, 70.0, [2049, 64])),
            refused(
                20,
                R::DeviceLimit {
                    size: [2049, 64],
                    max_side: 2048
                }
            )
        );
        assert_eq!(small.push(view(21, 70.0, [2048, 2048])), Ok(()));
        // A loading or title frame refuses the views asked for it.
        small.refuse_all(R::TitleScreen);
        assert!(small.views().is_empty());
        assert_eq!(small.logged(), 2);
    }

    /// Eight views are taken and the ninth refused; at 4096 a side, four fill the pixel budget
    /// and the fifth is refused with what it asked and what was left.
    #[test]
    fn eight_views_and_the_pixel_budget() {
        let mut views = WorldViews::new(MAX_WORLD_VIEW_SIDE);
        for id in 10..18 {
            assert_eq!(views.push(view(id, 70.0, [640, 360])), Ok(()));
        }
        assert_eq!(views.push(view(18, 70.0, [1, 1])), refused(18, R::TooMany));
        assert_eq!(ids(&views), [10, 11, 12, 13, 14, 15, 16, 17]);

        let mut views = WorldViews::new(MAX_WORLD_VIEW_SIDE);
        for id in 1..4 {
            assert_eq!(views.push(view(id, 70.0, [4096, 4096])), Ok(()));
        }
        assert_eq!(views.push(view(4, 70.0, [4096, 4095])), Ok(()));
        assert_eq!(
            views.push(view(5, 70.0, [4096, 2])),
            refused(
                5,
                R::PixelBudget {
                    pixels: 8192,
                    left: 4096
                }
            )
        );
        assert_eq!(views.push(view(6, 70.0, [4096, 1])), Ok(()));
        assert_eq!(
            views.push(view(7, 70.0, [1, 1])),
            refused(7, R::PixelBudget { pixels: 1, left: 0 })
        );
        assert_eq!(ids(&views), [1, 2, 3, 4, 6]);
        // a new frame starts the budget again
        views.begin(MAX_WORLD_VIEW_SIDE);
        assert_eq!(views.push(view(7, 70.0, [4096, 4096])), Ok(()));
    }

    /// Children draw before their parents and grandchildren before both, siblings in the order
    /// pushed, each at its depth; a parent not pushed first, refused or the view itself
    /// refuses the child by name, and depth 6 is refused.
    #[test]
    fn views_seen_in_views_draw_first() {
        let mut views = WorldViews::new(MAX_WORLD_VIEW_SIDE);
        let small = |id| view(id, 70.0, [64, 64]);
        assert_eq!(views.push(small(1)), Ok(()));
        assert_eq!(views.push(small(2).with_parent(1)), Ok(()));
        assert_eq!(views.push(small(3)), Ok(()));
        assert_eq!(views.push(small(4).with_parent(2)), Ok(()));
        assert_eq!(views.push(small(5).with_parent(1)), Ok(()));
        // a parent pushed after its child, the view's own id, a refused parent
        assert_eq!(
            views.push(small(6).with_parent(7)),
            refused(6, R::MissingParent(7))
        );
        assert_eq!(views.push(small(7)), Ok(()));
        assert_eq!(
            views.push(small(8).with_parent(8)),
            refused(8, R::MissingParent(8))
        );
        assert_eq!(
            views.push(view(9, 0.5, [64, 64])),
            refused(9, R::FieldOfView(0.5))
        );
        assert_eq!(
            views.push(small(10).with_parent(9)),
            refused(10, R::MissingParent(9))
        );
        views.order();
        assert_eq!(ids(&views), [4, 2, 5, 1, 3, 7]);
        let depths: Vec<u32> = (0..6).map(|k| views.depth(k)).collect();
        assert_eq!(depths, [3, 2, 2, 1, 1, 1]);
        let parents: Vec<_> = views.views().iter().map(|v| v.parent).collect();
        assert_eq!(parents, [Some(2), Some(1), Some(1), None, None, None]);

        // a chain of five is taken, the sixth refused
        let mut chain = WorldViews::new(MAX_WORLD_VIEW_SIDE);
        assert_eq!(chain.push(small(20)), Ok(()));
        for id in 21..25 {
            assert_eq!(chain.push(small(id).with_parent(id - 1)), Ok(()));
        }
        assert_eq!(
            chain.push(small(25).with_parent(24)),
            refused(25, R::TooDeep)
        );
        chain.order();
        assert_eq!(ids(&chain), [24, 23, 22, 21, 20]);
        assert_eq!(chain.depth(0), 5);
    }

    /// A view refused the same way every frame is logged once; the same view refused another
    /// way, or another view, is logged again.
    #[test]
    fn each_refusal_is_logged_once() {
        let mut views = WorldViews::new(MAX_WORLD_VIEW_SIDE);
        for _ in 0..3 {
            views.begin(MAX_WORLD_VIEW_SIDE);
            assert!(views.push(view(1, 0.5, [64, 64])).is_err());
            assert!(views.push(view(1, 0.25, [64, 64])).is_err());
        }
        assert_eq!(views.logged(), 1);
        assert!(views.push(view(1, 70.0, [0, 0])).is_err());
        assert!(views.push(view(2, 0.5, [64, 64])).is_err());
        assert_eq!(views.logged(), 3);
        // past 64 kept, nothing more is kept or logged
        for id in 100..200 {
            let _ = views.push(view(id, 0.5, [64, 64]));
        }
        assert_eq!(views.logged(), 64);
    }

    /// The player's own level, named explicitly, resolves like no level at all: taken, and
    /// nothing else of the view changes.
    #[test]
    fn the_players_own_level_named_resolves_like_none() {
        let own = LevelRef::new(WorldEpoch::new(7), DimensionKey::OVERWORLD);
        let mut views = WorldViews::new(MAX_WORLD_VIEW_SIDE);
        views.set_own_level(Some(own));
        let plain = view(1, 70.0, [64, 64]);
        assert_eq!(views.push(plain), Ok(()));
        assert_eq!(views.push(view(2, 70.0, [64, 64]).with_level(None)), Ok(()));
        let named = view(3, 70.0, [64, 64]).with_level(Some(own));
        assert_eq!(views.push(named), Ok(()));
        assert_eq!(
            views.views()[2],
            WorldView {
                id: 3,
                level: Some(own),
                ..plain
            }
        );
        assert_eq!(ids(&views), [1, 2, 3]);
    }

    /// A viewed level the server keeps open is not the player's own, and no view of it is drawn
    /// yet: refused as `Level`, logged once, and the views of the player's level are unaffected.
    #[test]
    fn an_open_viewed_level_is_refused() {
        let own = LevelRef::new(WorldEpoch::new(7), DimensionKey::OVERWORLD);
        let nether = LevelRef::new(WorldEpoch::new(7), DimensionKey::NETHER);
        let mut views = WorldViews::new(MAX_WORLD_VIEW_SIDE);
        for _ in 0..3 {
            views.begin(MAX_WORLD_VIEW_SIDE);
            views.set_own_level(Some(own));
            let viewed = view(1, 70.0, [64, 64]).with_level(Some(nether));
            assert_eq!(views.push(viewed), refused(1, R::Level));
            assert_eq!(views.push(view(2, 70.0, [64, 64])), Ok(()));
        }
        assert_eq!(ids(&views), [2]);
        assert_eq!(views.logged(), 1);
    }

    /// A handle from an earlier epoch (login, reconfiguration, a respawn into another level)
    /// is refused, whether it names the dimension the player is in or not; before login every
    /// named level is.
    #[test]
    fn a_stale_level_handle_is_refused() {
        let old = LevelRef::new(WorldEpoch::new(7), DimensionKey::OVERWORLD);
        let now = LevelRef::new(WorldEpoch::new(8), DimensionKey::OVERWORLD);
        let mut views = WorldViews::new(MAX_WORLD_VIEW_SIDE);
        views.set_own_level(Some(now));
        let stale = view(1, 70.0, [64, 64]).with_level(Some(old));
        assert_eq!(views.push(stale), refused(1, R::Level));
        // The next frame resolves again: the current handle is taken.
        views.begin(MAX_WORLD_VIEW_SIDE);
        assert_eq!(
            views.push(view(1, 70.0, [64, 64]).with_level(Some(now))),
            Ok(())
        );
        views.begin(MAX_WORLD_VIEW_SIDE);
        views.set_own_level(None);
        assert_eq!(
            views.push(view(1, 70.0, [64, 64]).with_level(Some(now))),
            refused(1, R::Level)
        );
    }
}
