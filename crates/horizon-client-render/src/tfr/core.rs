//! CPU-only TFR state machine and frame decisions (no GPU types).
//!
//! Deterministic for identical inputs. Spec: `TFR_DESCRIPTION.md`.

pub use super::config::TfrConfig;

/// Absolute world-space camera sample and camera-relative matrices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraSample {
    /// Absolute world-space camera/eye position (f64 for precision in cuts/reprojection setup).
    pub world_pos: [f64; 3],
    /// Yaw in degrees (Horizon camera convention).
    pub yaw_deg: f32,
    /// Pitch in degrees (Horizon camera convention).
    pub pitch_deg: f32,
    /// Camera-relative transforms in Horizon's renderer convention (column-major in storage).
    pub view: [[f32; 4]; 4],
    pub proj: [[f32; 4]; 4],
    pub view_proj: [[f32; 4]; 4],
    pub inv_view_proj: [[f32; 4]; 4],
}

/// Stable identity of a loaded world instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WorldKey {
    pub dimension: i32,
    pub generation: u64,
}

impl WorldKey {
    /// No world has been announced yet; never eligible for TFR.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            dimension: -1,
            generation: 0,
        }
    }

    #[must_use]
    pub const fn is_known(self) -> bool {
        self.dimension >= 0 && self.generation > 0
    }
}

/// Numeric id for a dimension identifier, for `WorldKey::dimension`.
///
/// Vanilla dimensions get stable small ids; other dimensions get a 64-bit FNV-1a hash of their id,
/// masked to a non-negative `i32` (so `WorldKey::is_known` accepts it) and kept out of `0..=2` so a
/// custom dimension cannot be mistaken for one of the vanilla ones. FNV-1a is fixed, unlike
/// `DefaultHasher`, so the id stays the same across Rust versions.
#[must_use]
pub fn dimension_id(id: &str) -> i32 {
    match id {
        "minecraft:overworld" | "overworld" => 0,
        "minecraft:the_nether" | "the_nether" | "nether" => 1,
        "minecraft:the_end" | "the_end" | "end" => 2,
        other => {
            let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
            for byte in other.bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
            let hashed = (hash & 0x7fff_ffff) as i32;
            if (0..=2).contains(&hashed) { 3 } else { hashed }
        }
    }
}

/// CPU-side world epoch: the dimension and the generation of the loaded world instance.
///
/// Kept outside the renderer so callers can advance it on world load/reload/dimension travel and
/// pass the resulting `WorldKey` to the renderer, which invalidates history when it changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldEpoch {
    dimension: i32,
    generation: u64,
}

impl WorldEpoch {
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            dimension: -1,
            generation: 0,
        }
    }

    #[must_use]
    pub const fn is_known(self) -> bool {
        self.dimension >= 0 && self.generation > 0
    }

    /// Observe a dimension. A new dimension (or the first known one) starts a new generation.
    /// Returns whether the dimension changed.
    pub fn observe(&mut self, dimension: i32) -> bool {
        if !self.is_known() || self.dimension != dimension {
            self.dimension = dimension;
            self.generation = self.generation.saturating_add(1);
            true
        } else {
            false
        }
    }

    /// A reload of the same world still makes its history stale.
    pub fn reload(&mut self) {
        if self.is_known() {
            self.generation = self.generation.saturating_add(1);
        }
    }

    #[must_use]
    pub const fn key(self) -> WorldKey {
        WorldKey {
            dimension: self.dimension,
            generation: self.generation,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct FrameSize {
    pub width: u32,
    pub height: u32,
}

impl FrameSize {
    #[must_use]
    pub const fn valid(self) -> bool {
        self.width > 0 && self.height > 0
    }
}

/// Scene render size from presentation size and internal scale.
///
/// Matches Horizon `world_post::sizes_with_upscale` render-size formula 1:1:
/// `max(1, round(present * scale).clamp(1, 8192))` per axis.
#[must_use]
pub fn scene_size(present: FrameSize, scale: f32) -> FrameSize {
    let scale = if scale.is_finite() { scale } else { 1.0 };
    let axis = |p: u32| -> u32 {
        let v = (p as f32 * scale).round();
        if !v.is_finite() {
            return 1;
        }
        (v as u32).clamp(1, 8192)
    };
    FrameSize {
        width: axis(present.width),
        height: axis(present.height),
    }
}

/// Spec scene size without device cap: `max(1, round(present * scale))` per axis.
#[must_use]
pub fn scene_size_unclamped(present: FrameSize, scale: f32) -> FrameSize {
    let scale = if scale.is_finite() { scale } else { 1.0 };
    let axis = |p: u32| -> u32 {
        let v = (p as f32 * scale).round();
        if !v.is_finite() || v <= 1.0 {
            1
        } else {
            v.min(u32::MAX as f32) as u32
        }
    };
    FrameSize {
        width: axis(present.width),
        height: axis(present.height),
    }
}

/// Motion-field size: `0.5 * scene`, minimum 1 per axis (floor).
#[must_use]
pub fn motion_field_size(scene: FrameSize) -> FrameSize {
    FrameSize {
        width: (scene.width / 2).max(1),
        height: (scene.height / 2).max(1),
    }
}

/// Which history source the current mode requires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistorySourceMode {
    DirectScene,
    PreparedRequired,
}

/// Per-frame inputs for the TFR decision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameContext {
    /// A world is active and renderable.
    pub world_active: bool,
    /// Meaningful only when `world_active` is true.
    pub world: WorldKey,
    /// Isolated scene render size.
    pub scene_size: FrameSize,
    /// Presentation/output size.
    pub present_size: FrameSize,
    pub camera: CameraSample,
    /// Valid history exists for the same world/sizes/mode/format.
    pub history_ready: bool,
    /// Prepared history is valid for the current history snapshot.
    pub prepared_history_ready: bool,
    /// Required source mode for this frame/path.
    pub history_source_mode: HistorySourceMode,
    /// Pipelines/bindings/resources are available for the required path.
    pub deps_ready: bool,
    /// Unsupported shader-pack/native-post state for isolated TFR.
    pub unsupported_pack_state: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameKind {
    Real,
    Synthetic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RealReason {
    Disabled,
    NoActiveWorld,
    /// The frame's `WorldKey` is not known (`WorldKey::unknown()`).
    UnknownWorld,
    InvalidSize,
    UnsupportedPack,
    NoHistory,
    WorldMismatch,
    SizeMismatch,
    SourceModeMismatch,
    PreparedHistoryMissing,
    MissingDeps,
    LimitReached,
    CutDistance,
    CutAngle,
    Invalidated,
    ErrorFallback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Real { reason: RealReason },
    Synthetic,
}

/// Metadata committed after a fully successful real frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RealFrameMeta {
    pub world: WorldKey,
    pub scene_size: FrameSize,
    pub present_size: FrameSize,
    pub camera: CameraSample,
    pub history_source_mode: HistorySourceMode,
    /// Whether prepared history was produced and is valid for this snapshot.
    pub prepared_history_ready: bool,
    /// `TfrState::config_epoch()` when this frame started. A commit from an earlier epoch (the
    /// config was replaced while the frame was in flight) is dropped.
    pub config_epoch: u64,
}

/// Meaningful-motion thresholds (fixed, below cut ranges). Recompute-only, never block synthetic.
pub const MOTION_POS_EPS: f64 = 0.001;
pub const MOTION_POS_EPS2: f64 = MOTION_POS_EPS * MOTION_POS_EPS;
pub const MOTION_ANGLE_EPS_DEG: f32 = 0.01;

#[must_use]
pub fn wrap_degrees(delta: f32) -> f32 {
    let mut d = delta % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d <= -180.0 {
        d += 360.0;
    }
    d
}

#[must_use]
pub fn clamp_pitch_deg(pitch: f32) -> f32 {
    pitch.clamp(-90.0, 90.0)
}

#[must_use]
pub fn angle_metric_deg(a: &CameraSample, b: &CameraSample) -> f32 {
    let dyaw = wrap_degrees(a.yaw_deg - b.yaw_deg);
    let dpitch = clamp_pitch_deg(a.pitch_deg) - clamp_pitch_deg(b.pitch_deg);
    (dyaw * dyaw + dpitch * dpitch).sqrt()
}

#[must_use]
pub fn distance_sq(a: &CameraSample, b: &CameraSample) -> f64 {
    let dx = a.world_pos[0] - b.world_pos[0];
    let dy = a.world_pos[1] - b.world_pos[1];
    let dz = a.world_pos[2] - b.world_pos[2];
    dx * dx + dy * dy + dz * dz
}

#[must_use]
pub fn meaningful_motion(a: &CameraSample, b: &CameraSample) -> bool {
    distance_sq(a, b) > MOTION_POS_EPS2 || angle_metric_deg(a, b) > MOTION_ANGLE_EPS_DEG
}

#[derive(Clone, Debug)]
struct HistoryMeta {
    world: WorldKey,
    scene_size: FrameSize,
    present_size: FrameSize,
    camera: CameraSample,
    source_mode: HistorySourceMode,
    prepared_ready: bool,
}

/// CPU-only TFR state.
#[derive(Clone, Debug, Default)]
pub struct TfrState {
    cfg: TfrConfig,
    history: Option<HistoryMeta>,
    /// Number of successful synthetic frames since last successful real commit (0 or 1).
    synthetics_since_real: u8,
    /// History invalidated and unusable until a successful real commit.
    invalidated: bool,
    /// Advances each time a different config replaces the current one.
    config_epoch: u64,
}

impl TfrState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Invalidate history immediately.
    pub fn invalidate(&mut self, _reason: &'static str) {
        self.history = None;
        self.invalidated = true;
        // Keep synthetics_since_real irrelevant while no valid history; real required.
        self.synthetics_since_real = 0;
    }

    /// Apply a config change and invalidate history.
    pub fn on_config_changed(&mut self, new_cfg: TfrConfig) {
        let new_cfg = new_cfg.sanitized();
        if self.cfg != new_cfg {
            self.invalidate("config changed");
            self.config_epoch = self.config_epoch.wrapping_add(1);
        }
        self.cfg = new_cfg;
    }

    /// The current config's epoch, for `RealFrameMeta::config_epoch`.
    #[must_use]
    pub fn config_epoch(&self) -> u64 {
        self.config_epoch
    }

    /// Source/size/mode-affecting change: invalidate before next decision.
    pub fn on_source_or_size_changed(&mut self) {
        self.invalidate("source or size changed");
    }

    /// Decision for this frame with state reconciliation (may invalidate stale history/config).
    ///
    /// Callers should still apply `on_config_changed` / `on_source_or_size_changed` before this
    /// when they know changes occurred; this method remains defensive.
    #[must_use]
    pub fn begin_frame(&mut self, cfg: TfrConfig, ctx: FrameContext) -> Decision {
        let cfg = cfg.sanitized();
        // Config change invalidates history before decision.
        if self.cfg != cfg {
            self.invalidate("config mismatch");
            self.config_epoch = self.config_epoch.wrapping_add(1);
            self.cfg = cfg;
        }

        if !cfg.enabled {
            return Decision::Real {
                reason: RealReason::Disabled,
            };
        }
        if !ctx.world_active {
            // No active world: history cannot be trusted for future use.
            self.invalidate("no active world");
            return Decision::Real {
                reason: RealReason::NoActiveWorld,
            };
        }
        if !ctx.world.is_known() {
            // Two unknown keys compare equal without naming the same world.
            self.invalidate("unknown world");
            return Decision::Real {
                reason: RealReason::UnknownWorld,
            };
        }
        if !ctx.scene_size.valid() || !ctx.present_size.valid() {
            self.invalidate("invalid size");
            return Decision::Real {
                reason: RealReason::InvalidSize,
            };
        }
        if ctx.unsupported_pack_state {
            // Keep state conservative; synthetic path unsupported.
            self.invalidate("unsupported pack state");
            return Decision::Real {
                reason: RealReason::UnsupportedPack,
            };
        }
        if self.invalidated || self.history.is_none() || !ctx.history_ready {
            if !ctx.history_ready {
                // Explicitly reported as not ready: do not keep stale committed history.
                self.history = None;
                self.invalidated = true;
                self.synthetics_since_real = 0;
            }
            return Decision::Real {
                reason: RealReason::NoHistory,
            };
        }
        let Some(h) = &self.history else {
            return Decision::Real {
                reason: RealReason::NoHistory,
            };
        };
        if h.world != ctx.world {
            self.invalidate("world mismatch");
            return Decision::Real {
                reason: RealReason::WorldMismatch,
            };
        }
        if h.scene_size != ctx.scene_size || h.present_size != ctx.present_size {
            self.invalidate("size mismatch");
            return Decision::Real {
                reason: RealReason::SizeMismatch,
            };
        }
        if h.source_mode != ctx.history_source_mode {
            self.invalidate("source mode mismatch");
            return Decision::Real {
                reason: RealReason::SourceModeMismatch,
            };
        }
        if ctx.history_source_mode == HistorySourceMode::PreparedRequired
            && !(ctx.prepared_history_ready && h.prepared_ready)
        {
            return Decision::Real {
                reason: RealReason::PreparedHistoryMissing,
            };
        }
        if !ctx.deps_ready {
            return Decision::Real {
                reason: RealReason::MissingDeps,
            };
        }
        if self.synthetics_since_real >= 1 {
            return Decision::Real {
                reason: RealReason::LimitReached,
            };
        }

        let dist2 = distance_sq(&ctx.camera, &h.camera);
        let cut_d2 = (cfg.cut_distance as f64) * (cfg.cut_distance as f64);
        if dist2 > cut_d2 {
            return Decision::Real {
                reason: RealReason::CutDistance,
            };
        }
        let ang = angle_metric_deg(&ctx.camera, &h.camera);
        if ang > cfg.cut_angle_deg {
            return Decision::Real {
                reason: RealReason::CutAngle,
            };
        }

        Decision::Synthetic
    }

    /// Commit a fully successful real frame with atomic history update.
    ///
    /// A frame that started under a config since replaced (a stale `meta.config_epoch`) is
    /// dropped: its history belongs to the old config and must not become valid for the new one.
    pub fn commit_real(&mut self, meta: RealFrameMeta) {
        if meta.config_epoch != self.config_epoch {
            return;
        }
        debug_assert!(
            !self.invalidated || self.cfg.enabled,
            "commit_real after invalidation is only valid as part of a successful real recovery path"
        );
        self.history = Some(HistoryMeta {
            world: meta.world,
            scene_size: meta.scene_size,
            present_size: meta.present_size,
            camera: meta.camera,
            source_mode: meta.history_source_mode,
            prepared_ready: meta.prepared_history_ready,
        });
        self.synthetics_since_real = 0;
        self.invalidated = false;
    }

    /// Commit a fully successful synthetic frame.
    pub fn commit_synthetic(&mut self) {
        debug_assert!(
            self.history.is_some() && !self.invalidated && self.synthetics_since_real == 0,
            "commit_synthetic requires a valid real-anchored history and no prior synthetic"
        );
        if self.history.is_some() && !self.invalidated {
            self.synthetics_since_real = 1;
        }
    }

    /// Real path taken as fallback/error recovery without a valid new history commit.
    pub fn note_fallback_real(&mut self) {
        self.invalidate("fallback real");
    }

    #[must_use]
    pub fn synthetics_since_real(&self) -> u8 {
        self.synthetics_since_real
    }

    #[must_use]
    pub fn has_valid_history(&self) -> bool {
        self.history.is_some() && !self.invalidated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(pos: [f64; 3], yaw: f32, pitch: f32) -> CameraSample {
        CameraSample {
            world_pos: pos,
            yaw_deg: yaw,
            pitch_deg: pitch,
            view: [[0.0; 4]; 4],
            proj: [[0.0; 4]; 4],
            view_proj: [[0.0; 4]; 4],
            inv_view_proj: [[0.0; 4]; 4],
        }
    }

    fn base_ctx() -> FrameContext {
        FrameContext {
            world_active: true,
            world: WorldKey {
                dimension: 0,
                generation: 1,
            },
            scene_size: FrameSize {
                width: 1920,
                height: 1080,
            },
            present_size: FrameSize {
                width: 1920,
                height: 1080,
            },
            camera: cam([0.0, 64.0, 0.0], 0.0, 0.0),
            history_ready: true,
            prepared_history_ready: true,
            history_source_mode: HistorySourceMode::DirectScene,
            deps_ready: true,
            unsupported_pack_state: false,
        }
    }

    fn enabled_cfg() -> TfrConfig {
        TfrConfig {
            enabled: true,
            ..Default::default()
        }
        .sanitized()
    }

    fn commit_base(s: &mut TfrState, ctx: FrameContext) {
        s.commit_real(RealFrameMeta {
            world: ctx.world,
            scene_size: ctx.scene_size,
            present_size: ctx.present_size,
            camera: ctx.camera,
            history_source_mode: ctx.history_source_mode,
            prepared_history_ready: matches!(
                ctx.history_source_mode,
                HistorySourceMode::DirectScene
            ) || ctx.prepared_history_ready,
            config_epoch: s.config_epoch(),
        });
    }

    #[test]
    fn disabled_gives_real() {
        let mut s = TfrState::new();
        let cfg = TfrConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(matches!(
            s.begin_frame(cfg, base_ctx()),
            Decision::Real {
                reason: RealReason::Disabled
            }
        ));
    }

    #[test]
    fn alternates_real_then_synthetic_then_real() {
        let mut s = TfrState::new();
        let cfg = enabled_cfg();
        s.on_config_changed(cfg);
        let ctx = base_ctx();
        assert!(matches!(
            s.begin_frame(cfg, ctx),
            Decision::Real {
                reason: RealReason::NoHistory
            }
        ));
        commit_base(&mut s, ctx);
        assert_eq!(s.begin_frame(cfg, ctx), Decision::Synthetic);
        s.commit_synthetic();
        assert!(matches!(
            s.begin_frame(cfg, ctx),
            Decision::Real {
                reason: RealReason::LimitReached
            }
        ));
    }

    #[test]
    fn distance_cut() {
        let mut s = TfrState::new();
        let mut cfg = enabled_cfg();
        cfg.cut_distance = 1.0;
        s.on_config_changed(cfg);
        let ctx = base_ctx();
        commit_base(&mut s, ctx);
        let mut far = ctx;
        far.camera.world_pos = [2.0, 64.0, 0.0];
        assert!(matches!(
            s.begin_frame(cfg, far),
            Decision::Real {
                reason: RealReason::CutDistance
            }
        ));
    }

    #[test]
    fn angle_cut_l2() {
        let mut s = TfrState::new();
        let mut cfg = enabled_cfg();
        cfg.cut_angle_deg = 10.0;
        s.on_config_changed(cfg);
        let ctx = base_ctx();
        commit_base(&mut s, ctx);
        let mut turned = ctx;
        turned.camera.yaw_deg = 8.0;
        turned.camera.pitch_deg = 8.0; // sqrt(128)>10
        assert!(matches!(
            s.begin_frame(cfg, turned),
            Decision::Real {
                reason: RealReason::CutAngle
            }
        ));
        let mut ok = ctx;
        ok.camera.yaw_deg = 3.0;
        ok.camera.pitch_deg = 4.0; // 5
        assert_eq!(s.begin_frame(cfg, ok), Decision::Synthetic);
    }

    #[test]
    fn prepared_required_blocks_synthetic_when_missing() {
        let mut s = TfrState::new();
        let cfg = enabled_cfg();
        s.on_config_changed(cfg);
        let mut ctx = base_ctx();
        ctx.history_source_mode = HistorySourceMode::PreparedRequired;
        ctx.prepared_history_ready = true;
        commit_base(&mut s, ctx);
        ctx.prepared_history_ready = false;
        assert!(matches!(
            s.begin_frame(cfg, ctx),
            Decision::Real {
                reason: RealReason::PreparedHistoryMissing
            }
        ));
        // Committed history remains valid; transient readiness in context blocks synthetic.
        assert!(s.has_valid_history());
        ctx.prepared_history_ready = true;
        assert_eq!(s.begin_frame(cfg, ctx), Decision::Synthetic);
    }

    #[test]
    fn prepared_direct_scene_ignores_prepared_flag() {
        let mut s = TfrState::new();
        let cfg = enabled_cfg();
        s.on_config_changed(cfg);
        let mut ctx = base_ctx();
        ctx.history_source_mode = HistorySourceMode::DirectScene;
        ctx.prepared_history_ready = false;
        commit_base(&mut s, ctx);
        ctx.prepared_history_ready = false;
        assert_eq!(s.begin_frame(cfg, ctx), Decision::Synthetic);
    }

    #[test]
    fn world_change_invalidates() {
        let mut s = TfrState::new();
        let cfg = enabled_cfg();
        s.on_config_changed(cfg);
        let ctx = base_ctx();
        commit_base(&mut s, ctx);
        let mut other = ctx;
        other.world.generation = 2;
        assert!(matches!(
            s.begin_frame(cfg, other),
            Decision::Real {
                reason: RealReason::WorldMismatch
            }
        ));
        assert!(!s.has_valid_history());
    }

    #[test]
    fn unknown_world_never_goes_synthetic() {
        let mut s = TfrState::new();
        let cfg = enabled_cfg();
        s.on_config_changed(cfg);
        let mut ctx = base_ctx();
        ctx.world = WorldKey::unknown();
        commit_base(&mut s, ctx);
        assert_eq!(
            s.begin_frame(cfg, ctx),
            Decision::Real {
                reason: RealReason::UnknownWorld
            }
        );
    }

    #[test]
    fn commit_started_under_replaced_config_is_dropped() {
        let mut s = TfrState::new();
        let cfg = enabled_cfg();
        s.on_config_changed(cfg);
        let ctx = base_ctx();
        let stale_epoch = s.config_epoch();
        s.on_config_changed(TfrConfig {
            cut_angle_deg: 30.0,
            ..cfg
        });
        s.commit_real(RealFrameMeta {
            world: ctx.world,
            scene_size: ctx.scene_size,
            present_size: ctx.present_size,
            camera: ctx.camera,
            history_source_mode: ctx.history_source_mode,
            prepared_history_ready: true,
            config_epoch: stale_epoch,
        });
        assert!(!s.has_valid_history());
    }

    #[test]
    fn present_size_mismatch_invalidates() {
        let mut s = TfrState::new();
        let cfg = enabled_cfg();
        s.on_config_changed(cfg);
        let ctx = base_ctx();
        commit_base(&mut s, ctx);
        let mut other = ctx;
        other.present_size = FrameSize {
            width: 1280,
            height: 720,
        };
        assert!(matches!(
            s.begin_frame(cfg, other),
            Decision::Real {
                reason: RealReason::SizeMismatch
            }
        ));
        assert!(!s.has_valid_history());
    }

    #[test]
    fn no_active_world() {
        let mut s = TfrState::new();
        let cfg = enabled_cfg();
        s.on_config_changed(cfg);
        let ctx = base_ctx();
        commit_base(&mut s, ctx);
        let mut none = ctx;
        none.world_active = false;
        assert!(matches!(
            s.begin_frame(cfg, none),
            Decision::Real {
                reason: RealReason::NoActiveWorld
            }
        ));
        assert!(!s.has_valid_history());
    }

    #[test]
    fn on_config_changed_invalidates() {
        let mut s = TfrState::new();
        let cfg = enabled_cfg();
        s.on_config_changed(cfg);
        let ctx = base_ctx();
        commit_base(&mut s, ctx);
        let mut cfg2 = cfg;
        cfg2.reprojection_strength = 0.5;
        s.on_config_changed(cfg2);
        assert!(matches!(
            s.begin_frame(cfg2, ctx),
            Decision::Real {
                reason: RealReason::NoHistory
            }
        ));
    }

    #[test]
    fn on_source_or_size_changed_invalidates() {
        let mut s = TfrState::new();
        let cfg = enabled_cfg();
        s.on_config_changed(cfg);
        let ctx = base_ctx();
        commit_base(&mut s, ctx);
        s.on_source_or_size_changed();
        assert!(matches!(
            s.begin_frame(cfg, ctx),
            Decision::Real {
                reason: RealReason::NoHistory
            }
        ));
    }

    #[test]
    fn note_fallback_real_blocks_synthetic() {
        let mut s = TfrState::new();
        let cfg = enabled_cfg();
        s.on_config_changed(cfg);
        let ctx = base_ctx();
        commit_base(&mut s, ctx);
        s.note_fallback_real();
        assert!(matches!(
            s.begin_frame(cfg, ctx),
            Decision::Real {
                reason: RealReason::NoHistory
            }
        ));
    }

    #[test]
    fn history_not_ready_clears() {
        let mut s = TfrState::new();
        let cfg = enabled_cfg();
        s.on_config_changed(cfg);
        let ctx = base_ctx();
        commit_base(&mut s, ctx);
        let mut gone = ctx;
        gone.history_ready = false;
        assert!(matches!(
            s.begin_frame(cfg, gone),
            Decision::Real {
                reason: RealReason::NoHistory
            }
        ));
        assert!(!s.has_valid_history());
    }

    #[test]
    fn size_rounding() {
        assert_eq!(
            scene_size(
                FrameSize {
                    width: 1920,
                    height: 1080
                },
                0.5
            ),
            FrameSize {
                width: 960,
                height: 540
            }
        );
        assert_eq!(
            scene_size(
                FrameSize {
                    width: 101,
                    height: 101
                },
                0.5
            ),
            FrameSize {
                width: 51,
                height: 51
            }
        );
        assert_eq!(
            scene_size(
                FrameSize {
                    width: 10,
                    height: 10
                },
                0.0
            ),
            FrameSize {
                width: 1,
                height: 1
            }
        );
        assert_eq!(
            scene_size(
                FrameSize {
                    width: 10000,
                    height: 10
                },
                2.0
            ),
            FrameSize {
                width: 8192,
                height: 20
            }
        );
        assert_eq!(
            scene_size_unclamped(
                FrameSize {
                    width: 10000,
                    height: 10
                },
                2.0
            ),
            FrameSize {
                width: 20000,
                height: 20
            }
        );
        assert_eq!(
            motion_field_size(FrameSize {
                width: 1,
                height: 3
            }),
            FrameSize {
                width: 1,
                height: 1
            }
        );
        assert_eq!(
            motion_field_size(FrameSize {
                width: 1920,
                height: 1080
            }),
            FrameSize {
                width: 960,
                height: 540
            }
        );
    }

    #[test]
    fn dimension_ids_are_stable_and_distinct() {
        assert_eq!(dimension_id("minecraft:overworld"), 0);
        assert_eq!(dimension_id("minecraft:the_nether"), 1);
        assert_eq!(dimension_id("minecraft:the_end"), 2);
        // A custom dimension must not alias a vanilla one, and must be deterministic.
        let custom = dimension_id("modded:dim");
        assert!(!(0..=2).contains(&custom));
        assert_eq!(custom, dimension_id("modded:dim"));
        assert_ne!(custom, dimension_id("modded:other"));
        // Every custom id is non-negative, so a world in it can be known. 64-bit FNV-1a of
        // "a" is the published 0xaf63dc4c8601ec8c; its low 31 bits are 0x0601ec8c.
        assert_eq!(dimension_id("a"), 0x0601_ec8c);
        for id in ["modded:dim", "modded:other", "x:y", "aether:the_aether"] {
            let mut epoch = WorldEpoch::unknown();
            epoch.observe(dimension_id(id));
            assert!(epoch.key().is_known(), "{id}");
        }
        assert!(!WorldKey::unknown().is_known());
    }

    #[test]
    fn world_epoch_tracks_dimension_and_reload() {
        let mut epoch = WorldEpoch::unknown();
        assert!(!epoch.is_known());
        // The first known world starts at generation 1.
        assert!(epoch.observe(0));
        assert_eq!(
            epoch.key(),
            WorldKey {
                dimension: 0,
                generation: 1
            }
        );
        // Staying in the same dimension does not move the key on its own.
        assert!(!epoch.observe(0));
        assert_eq!(
            epoch.key(),
            WorldKey {
                dimension: 0,
                generation: 1
            }
        );
        // A reload of the same world still invalidates its history.
        epoch.reload();
        assert_eq!(
            epoch.key(),
            WorldKey {
                dimension: 0,
                generation: 2
            }
        );
        // Travelling to another dimension changes both parts of the key.
        assert!(epoch.observe(1));
        assert_eq!(
            epoch.key(),
            WorldKey {
                dimension: 1,
                generation: 3
            }
        );
        assert!(!epoch.observe(1));
        // A later reload keeps the dimension and advances only the generation.
        epoch.reload();
        assert_eq!(
            epoch.key(),
            WorldKey {
                dimension: 1,
                generation: 4
            }
        );
    }
}
