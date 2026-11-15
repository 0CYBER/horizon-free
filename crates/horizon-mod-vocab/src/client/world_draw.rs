//! Untextured, unlit geometry submitted for one world view.

/// Whether world geometry is hidden by nearer world surfaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum WorldDepth {
    /// Test against the world's depth, without writing depth.
    Tested,
    /// Draw over world surfaces, without testing or writing depth.
    ThroughWalls,
}

/// A world-space line, colored with 0xAARRGGBB and measured in screen pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct WorldLine {
    /// Starting position in blocks, before camera subtraction.
    pub from: [f64; 3],
    /// Ending position in blocks, before camera subtraction.
    pub to: [f64; 3],
    /// Packed ARGB color, including alpha.
    pub color: u32,
    /// Finite, positive width in pixels.
    pub width: f32,
    /// Occlusion behavior.
    pub depth: WorldDepth,
}
impl WorldLine {
    /// Describe a line; the draw context validates it when submitting the batch.
    pub fn new(from: [f64; 3], to: [f64; 3], color: u32, width: f32, depth: WorldDepth) -> Self {
        Self {
            from,
            to,
            color,
            width,
            depth,
        }
    }
}

/// A two-sided, unlit quad. Corners run top left, bottom left, bottom right,
/// top right as seen from the front; alpha blends without writing depth.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct WorldQuad {
    /// Four world-space corners in blocks, before camera subtraction.
    pub corners: [[f64; 3]; 4],
    /// Packed 0xAARRGGBB color, including alpha.
    pub color: u32,
    /// Occlusion behavior.
    pub depth: WorldDepth,
}
impl WorldQuad {
    /// Describe a quad; the draw context validates it when submitting the batch.
    pub fn new(corners: [[f64; 3]; 4], color: u32, depth: WorldDepth) -> Self {
        Self {
            corners,
            color,
            depth,
        }
    }
}

/// Why a whole geometry batch was refused; nothing from that batch was appended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum DrawError {
    /// A coordinate or width is not finite, or a camera-relative coordinate overflows f32.
    NonFinite,
    /// A finite line width is zero or negative.
    InvalidWidth,
    /// The batch would exceed this view's setup-time capacity.
    CapacityExceeded,
}

/// Where in the world's frame a stage's geometry is drawn: NeoForge 26.3's
/// `RenderLevelStageEvent` stages Horizon draws, in the order the frame reaches them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum RenderLevelStage {
    /// After the sky, sun, moon and stars, before any terrain (`AfterSky`): behind
    /// everything else the world draws, for backdrops and atmosphere.
    AfterSky,
    /// After solid and cutout terrain, before entities and block entities
    /// (`AfterOpaqueBlocks`).
    AfterOpaqueBlocks,
    /// After translucent terrain such as water and stained glass, before translucent
    /// particles (`AfterTranslucentBlocks`).
    AfterTranslucentBlocks,
    /// After rain and snow, before the world border (`AfterWeather`).
    AfterWeather,
    /// After the rest of the world, before the first-person hand (`AfterLevel`): where
    /// `submit_custom_geometry` and the debug renderers draw.
    AfterLevel,
}
impl RenderLevelStage {
    /// Every stage, in the order the frame reaches them.
    pub const ALL: [Self; 5] = [
        Self::AfterSky,
        Self::AfterOpaqueBlocks,
        Self::AfterTranslucentBlocks,
        Self::AfterWeather,
        Self::AfterLevel,
    ];
}

/// Immutable, minimal extracted state for custom world geometry submission.
/// This initial view does not expose NeoForge's full level state or visible sections.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct WorldRenderView {
    /// Camera origin in world-space blocks. Subtraction precedes f32 conversion.
    pub camera: [f64; 3],
    /// The frame's interpolation fraction.
    pub partial_tick: f32,
    /// Level game time in ticks.
    pub game_time: i64,
}
impl WorldRenderView {
    /// Describe the extracted view.
    pub fn new(camera: [f64; 3], partial_tick: f32, game_time: i64) -> Self {
        Self {
            camera,
            partial_tick,
            game_time,
        }
    }
}

/// Queued primitive counts and fixed capacities, shared across both depth modes.
///
/// `instances_capacity` is the per-view budget of 16,384 model copies. It is reserved
/// when the view's storage is created and is never grown during submission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct WorldDrawUsage {
    /// Lines queued for this view after near-plane clipping.
    pub lines: u32,
    /// Maximum lines per view.
    pub line_capacity: u32,
    /// Quads queued for this view.
    pub quads: u32,
    /// Maximum quads per view.
    pub quad_capacity: u32,
    /// Model copies accepted for this view. Culled copies are not included.
    /// A call may pass at most [`instances_capacity`](Self::instances_capacity)
    /// copies; that bound is applied before culling.
    pub instances_used: u32,
    /// Maximum model copies per view, and the most copies one call may pass.
    pub instances_capacity: u32,
}
impl WorldDrawUsage {
    /// Describe the current usage and setup-time limits.
    pub fn new(
        lines: u32,
        line_capacity: u32,
        quads: u32,
        quad_capacity: u32,
        instances_used: u32,
        instances_capacity: u32,
    ) -> Self {
        Self {
            lines,
            line_capacity,
            quads,
            quad_capacity,
            instances_used,
            instances_capacity,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    struct Listener;
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn submit_custom_geometry(
            &self,
            _: &dyn ClientHost,
            event: &mut SubmitCustomGeometryEvent<'_>,
        ) {
            assert_eq!(event.view.game_time, 42);
            CALLS.fetch_add(1, Ordering::Relaxed);
        }
    }
    struct Empty;
    #[mod_api::__subscriptions]
    impl Hooks for Empty {}
    impl Host for Empty {}
    mod_api::impl_host_areas!(Empty);
    impl ClientHost for Empty {}
    mod_api::impl_client_host_areas!(Empty);
    impl WorldDrawContext for Empty {}
    #[test]
    fn world_geometry_dispatch_skips_unsubscribed_and_disabled_mods() {
        install(
            vec![
                ModEntry::new("geometry", Box::new(Listener)),
                ModEntry::new("empty", Box::new(Empty)),
            ]
            .into(),
        );
        let view = WorldRenderView::new([0.0; 3], 0.5, 42);
        dispatch_submit_custom_geometry(&Empty, &view, &mut Empty);
        assert_eq!(CALLS.load(Ordering::Relaxed), 1);
        assert!(set_mod_enabled("geometry", false));
        dispatch_submit_custom_geometry(&Empty, &view, &mut Empty);
        assert_eq!(CALLS.load(Ordering::Relaxed), 1);
    }
}
