//! TFR core conformance tests.
//!
//! Targets the CPU-only, GPU-free API in `horizon_client_render::tfr::{core,config}`.
//! Intended to run without a device or world renderer.
//!
//! Clean room: derived from the behavioural requirements only. Expected values come from
//! those requirements, not implementation literals (except where testing a documented API
//! contract/helper such as world_post-compatible sizing).

use horizon_client_render::tfr::config::TfrConfig;
use horizon_client_render::tfr::core::{
    CameraSample, Decision, FrameContext, FrameSize, HistorySourceMode, MOTION_ANGLE_EPS_DEG,
    MOTION_POS_EPS, MOTION_POS_EPS2, RealFrameMeta, RealReason, TfrState, WorldKey,
    angle_metric_deg, clamp_pitch_deg, distance_sq, meaningful_motion, motion_field_size,
    scene_size, scene_size_unclamped, wrap_degrees,
};

fn default_config() -> TfrConfig {
    TfrConfig::default().sanitized()
}

fn enabled_config() -> TfrConfig {
    TfrConfig {
        enabled: true,
        ..TfrConfig::default()
    }
    .sanitized()
}

fn world(generation: u64) -> WorldKey {
    WorldKey {
        dimension: 0,
        generation,
    }
}

fn camera(pos: [f64; 3], yaw_deg: f32, pitch_deg: f32) -> CameraSample {
    CameraSample {
        world_pos: pos,
        yaw_deg,
        pitch_deg,
        // Decision/state tests do not require specific matrices.
        view: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
        proj: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
        view_proj: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
        inv_view_proj: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }
}

fn base_ctx(cam: CameraSample) -> FrameContext {
    FrameContext {
        world_active: true,
        world: world(1),
        scene_size: FrameSize {
            width: 1280,
            height: 720,
        },
        present_size: FrameSize {
            width: 1280,
            height: 720,
        },
        camera: cam,
        history_ready: false,
        prepared_history_ready: true,
        history_source_mode: HistorySourceMode::DirectScene,
        deps_ready: true,
        unsupported_pack_state: false,
    }
}

fn ready_ctx(cam: CameraSample) -> FrameContext {
    let mut c = base_ctx(cam);
    c.history_ready = true;
    c
}

fn real_meta(
    state: &TfrState,
    world_key: WorldKey,
    cam: CameraSample,
    mode: HistorySourceMode,
) -> RealFrameMeta {
    RealFrameMeta {
        world: world_key,
        scene_size: FrameSize {
            width: 1280,
            height: 720,
        },
        present_size: FrameSize {
            width: 1280,
            height: 720,
        },
        camera: cam,
        history_source_mode: mode,
        prepared_history_ready: true,
        config_epoch: state.config_epoch(),
    }
}

fn commit_successful_real(state: &mut TfrState, w: WorldKey, cam: CameraSample) {
    let meta = real_meta(state, w, cam, HistorySourceMode::DirectScene);
    state.commit_real(meta);
}

fn assert_real(decision: Decision) -> RealReason {
    match decision {
        Decision::Real { reason } => reason,
        Decision::Synthetic => panic!("expected Real, got Synthetic"),
    }
}

fn assert_real_reason(decision: Decision, expected: RealReason) {
    assert_eq!(assert_real(decision), expected, "unexpected real reason");
}

fn assert_synthetic(decision: Decision) {
    assert!(
        matches!(decision, Decision::Synthetic),
        "expected Synthetic, got {decision:?}"
    );
}

// C-01: enabled default off
#[test]
fn config_defaults_match_spec() {
    let c = TfrConfig::default();
    assert!(!c.enabled);
    assert!((c.reprojection_strength - 1.0).abs() <= f32::EPSILON);
    assert!((c.history_stability - 0.06).abs() <= 1e-6);
    assert!((c.cut_distance - 9.25).abs() <= 1e-6);
    assert!((c.cut_angle_deg - 14.0).abs() <= 1e-6);
}

// C-02..C-05: clamping/ranges
#[test]
fn config_clamps_to_spec_ranges() {
    let c = TfrConfig {
        enabled: true,
        reprojection_strength: 2.0,
        history_stability: 0.9,
        cut_distance: 100.0,
        cut_angle_deg: 180.0,
    }
    .sanitized();
    assert!((c.reprojection_strength - 1.0).abs() <= f32::EPSILON);
    assert!((c.history_stability - 0.35).abs() <= 1e-6);
    assert!((c.cut_distance - 16.0).abs() <= 1e-6);
    assert!((c.cut_angle_deg - 90.0).abs() <= 1e-6);

    let low = TfrConfig {
        enabled: true,
        reprojection_strength: -1.0,
        history_stability: -0.1,
        cut_distance: 0.0,
        cut_angle_deg: 0.0,
    }
    .sanitized();
    assert!((low.reprojection_strength - 0.0).abs() <= f32::EPSILON);
    assert!((low.history_stability - 0.0).abs() <= 1e-6);
    assert!((low.cut_distance - 0.25).abs() <= 1e-6);
    assert!((low.cut_angle_deg - 5.0).abs() <= 1e-6);
}

// R-01/R-02/R-05 sizing helpers (spec-derived expectations).
#[test]
fn scene_size_unclamped_rounds_per_axis_and_enforces_min_one() {
    // Spec: round(present*scale) per axis, min 1.
    assert_eq!(
        scene_size_unclamped(
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
        scene_size_unclamped(
            FrameSize {
                width: 3,
                height: 3
            },
            0.5
        ),
        FrameSize {
            width: 2,
            height: 2
        } // 1.5 rounds to 2
    );
    assert_eq!(
        scene_size_unclamped(
            FrameSize {
                width: 3,
                height: 3
            },
            0.34
        ),
        FrameSize {
            width: 1,
            height: 1
        } // 1.02 rounds to 1
    );
    assert_eq!(
        scene_size_unclamped(
            FrameSize {
                width: 100,
                height: 100
            },
            0.001
        ),
        FrameSize {
            width: 1,
            height: 1
        }
    );
}

#[test]
fn motion_field_size_is_half_scene_min_one() {
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
    assert_eq!(
        motion_field_size(FrameSize {
            width: 3,
            height: 1
        }),
        FrameSize {
            width: 1,
            height: 1
        }
    );
}

// P-01, D-04: at most one synthetic after each successful real frame
#[test]
fn alternates_one_synthetic_after_each_successful_real() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    let w = world(1);

    assert_real_reason(state.begin_frame(cfg, base_ctx(cam)), RealReason::NoHistory);
    commit_successful_real(&mut state, w, cam);

    assert_synthetic(state.begin_frame(cfg, ready_ctx(cam)));
    state.commit_synthetic();

    assert_real_reason(
        state.begin_frame(cfg, ready_ctx(cam)),
        RealReason::LimitReached,
    );
    commit_successful_real(&mut state, w, cam);

    assert_synthetic(state.begin_frame(cfg, ready_ctx(cam)));
    state.commit_synthetic();

    assert_real_reason(
        state.begin_frame(cfg, ready_ctx(cam)),
        RealReason::LimitReached,
    );
}

// D-01: disabled => real, no synthetic
#[test]
fn disabled_never_synthetic() {
    let disabled = default_config();
    let enabled = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(enabled);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), cam);
    assert_real_reason(
        state.begin_frame(disabled, ready_ctx(cam)),
        RealReason::Disabled,
    );
}

// D-01: no active world
#[test]
fn no_active_world_requires_real() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), cam);
    let mut c = ready_ctx(cam);
    c.world_active = false;
    assert_real_reason(state.begin_frame(cfg, c), RealReason::NoActiveWorld);
}

// D-01, CR-07: world mismatch/changed
#[test]
fn world_change_forbids_synthetic() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), cam);
    let mut c = ready_ctx(cam);
    c.world = world(2);
    let reason = assert_real(state.begin_frame(cfg, c));
    assert!(matches!(
        reason,
        RealReason::WorldMismatch | RealReason::NoHistory | RealReason::Invalidated
    ));
}

// D-02: invalid sizes
#[test]
fn invalid_scene_size_requires_real() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), cam);
    let mut c = ready_ctx(cam);
    c.scene_size = FrameSize {
        width: 0,
        height: 720,
    };
    assert_real_reason(state.begin_frame(cfg, c), RealReason::InvalidSize);
}

#[test]
fn invalid_present_size_requires_real() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), cam);
    let mut c = ready_ctx(cam);
    c.present_size = FrameSize {
        width: 1280,
        height: 0,
    };
    assert_real_reason(state.begin_frame(cfg, c), RealReason::InvalidSize);
}

// R-06, CR-06: size mismatch
#[test]
fn scene_size_mismatch_requires_real() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), cam);
    let mut c = ready_ctx(cam);
    c.scene_size = FrameSize {
        width: 1920,
        height: 1080,
    };
    let reason = assert_real(state.begin_frame(cfg, c));
    assert!(matches!(
        reason,
        RealReason::SizeMismatch | RealReason::Invalidated
    ));
}

#[test]
fn present_size_mismatch_requires_real() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), cam);
    let mut c = ready_ctx(cam);
    c.present_size = FrameSize {
        width: 1920,
        height: 1080,
    };
    let reason = assert_real(state.begin_frame(cfg, c));
    assert!(matches!(
        reason,
        RealReason::SizeMismatch | RealReason::Invalidated
    ));
}

// D-03: missing dependencies => safe real
#[test]
fn missing_dependencies_requires_real() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), cam);
    let mut c = ready_ctx(cam);
    c.deps_ready = false;
    assert_real_reason(state.begin_frame(cfg, c), RealReason::MissingDeps);
}

// Shader pack / unsupported state => safe real fallback
#[test]
fn unsupported_pack_state_requires_real() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), cam);
    let mut c = ready_ctx(cam);
    c.unsupported_pack_state = true;
    assert_real_reason(state.begin_frame(cfg, c), RealReason::UnsupportedPack);
}

// Prepared history: required but not ready => synthetic forbidden.
#[test]
fn prepared_required_without_ready_forbids_synthetic() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    state.commit_real(real_meta(
        &state,
        world(1),
        cam,
        HistorySourceMode::PreparedRequired,
    ));
    let mut c = ready_ctx(cam);
    c.history_source_mode = HistorySourceMode::PreparedRequired;
    c.prepared_history_ready = false;
    assert_real_reason(
        state.begin_frame(cfg, c),
        RealReason::PreparedHistoryMissing,
    );
}

// Transient unready prepared context must not destroy a valid prepared history snapshot.
#[test]
fn prepared_missing_is_transient_when_history_still_valid() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    state.commit_real(real_meta(
        &state,
        world(1),
        cam,
        HistorySourceMode::PreparedRequired,
    ));
    let mut c = ready_ctx(cam);
    c.history_source_mode = HistorySourceMode::PreparedRequired;
    c.prepared_history_ready = false;
    assert_real_reason(
        state.begin_frame(cfg, c),
        RealReason::PreparedHistoryMissing,
    );
    c.prepared_history_ready = true;
    assert_synthetic(state.begin_frame(cfg, c));
}

// Direct scene mode: prepared readiness does not gate synthetic.
#[test]
fn direct_scene_ignores_prepared_history_readiness() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), cam);
    let mut c = ready_ctx(cam);
    c.history_source_mode = HistorySourceMode::DirectScene;
    c.prepared_history_ready = false;
    assert_synthetic(state.begin_frame(cfg, c));
}

// CM-01, CR-05: distance cut triggered when displacement exceeds threshold (>), not at equality.
#[test]
fn distance_at_threshold_allows_synthetic() {
    let mut cfg = enabled_config();
    cfg.cut_distance = 10.0;
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let ref_cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), ref_cam);
    let at = camera([10.0, 64.0, 0.0], 0.0, 0.0);
    assert_synthetic(state.begin_frame(cfg, ready_ctx(at)));
}

#[test]
fn distance_above_threshold_requires_real_cut() {
    let mut cfg = enabled_config();
    cfg.cut_distance = 10.0;
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let ref_cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), ref_cam);
    let above = camera([10.0001, 64.0, 0.0], 0.0, 0.0);
    assert_real_reason(
        state.begin_frame(cfg, ready_ctx(above)),
        RealReason::CutDistance,
    );
}

#[test]
fn distance_below_threshold_allows_synthetic() {
    let mut cfg = enabled_config();
    cfg.cut_distance = 10.0;
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let ref_cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), ref_cam);
    let below = camera([9.999, 64.0, 0.0], 0.0, 0.0);
    assert_synthetic(state.begin_frame(cfg, ready_ctx(below)));
}

// CM-02, CR-04: angle cut triggered when aggregated orientation delta exceeds threshold (>).
#[test]
fn angle_at_threshold_allows_synthetic() {
    let mut cfg = enabled_config();
    cfg.cut_angle_deg = 14.0;
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let ref_cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), ref_cam);
    let at = camera([0.0, 64.0, 0.0], 14.0, 0.0);
    assert_synthetic(state.begin_frame(cfg, ready_ctx(at)));
}

#[test]
fn angle_above_threshold_requires_real_cut() {
    let mut cfg = enabled_config();
    cfg.cut_angle_deg = 14.0;
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let ref_cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), ref_cam);
    let above = camera([0.0, 64.0, 0.0], 14.01, 0.0);
    assert_real_reason(
        state.begin_frame(cfg, ready_ctx(above)),
        RealReason::CutAngle,
    );
}

#[test]
fn angle_below_threshold_allows_synthetic() {
    let mut cfg = enabled_config();
    cfg.cut_angle_deg = 14.0;
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let ref_cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), ref_cam);
    let below = camera([0.0, 64.0, 0.0], 13.99, 0.0);
    assert_synthetic(state.begin_frame(cfg, ready_ctx(below)));
}

#[test]
fn yaw_wrap_small_delta_is_not_cut() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let ref_cam = camera([0.0, 64.0, 0.0], 179.0, 0.0);
    commit_successful_real(&mut state, world(1), ref_cam);
    let near_wrap = camera([0.0, 64.0, 0.0], -179.0, 0.0);
    assert_synthetic(state.begin_frame(cfg, ready_ctx(near_wrap)));
}

#[test]
fn pitch_limits_must_not_wrap_like_yaw() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let ref_cam = camera([0.0, 64.0, 0.0], 0.0, 89.0);
    commit_successful_real(&mut state, world(1), ref_cam);
    // 178 degrees apart in value, not a small orientation change.
    let flipped = camera([0.0, 64.0, 0.0], 0.0, -89.0);
    assert_real_reason(
        state.begin_frame(cfg, ready_ctx(flipped)),
        RealReason::CutAngle,
    );
}

#[test]
fn angle_metric_is_l2_of_wrapped_deltas() {
    let a = camera([0.0, 0.0, 0.0], 179.0, 10.0);
    let b = camera([0.0, 0.0, 0.0], -179.0, 13.0);
    let m = angle_metric_deg(&a, &b);
    // dyaw=2, dpitch=3 => sqrt(13)
    assert!((m - 13.0_f32.sqrt()).abs() < 1e-5, "got {m}");
}

#[test]
fn meaningful_motion_below_cut_does_not_forbid_synthetic() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let ref_cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), ref_cam);
    let moved = camera([0.01, 64.0, 0.0], 0.1, 0.0);
    assert!(meaningful_motion(&ref_cam, &moved));
    assert_synthetic(state.begin_frame(cfg, ready_ctx(moved)));
}

#[test]
fn no_meaningful_motion_still_allows_synthetic() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let ref_cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), ref_cam);
    let same = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    assert!(!meaningful_motion(&ref_cam, &same));
    assert_synthetic(state.begin_frame(cfg, ready_ctx(same)));
}

// CM-03: small motion below cut remains synthetic-eligible.
#[test]
fn small_motion_below_cut_allows_synthetic() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let ref_cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), ref_cam);
    let moved = camera([0.1, 64.0, 0.0], 1.0, 0.5);
    assert_synthetic(state.begin_frame(cfg, ready_ctx(moved)));
}

// Invalidation/config/source-size changes
#[test]
fn invalidate_clears_history() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), cam);
    state.invalidate("test");
    assert!(!state.has_valid_history());
    assert_real_reason(
        state.begin_frame(cfg, ready_ctx(cam)),
        RealReason::NoHistory,
    );
}

#[test]
fn config_change_invalidates_history() {
    let mut cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), cam);
    cfg.reprojection_strength = 0.5;
    state.on_config_changed(cfg);
    assert!(!state.has_valid_history());
}

#[test]
fn source_or_size_change_invalidates_history() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), cam);
    state.on_source_or_size_changed();
    assert!(!state.has_valid_history());
    assert_real_reason(
        state.begin_frame(cfg, ready_ctx(cam)),
        RealReason::NoHistory,
    );
}

// begin_frame is deterministic for unchanged state/context; synthetic slot is consumed by commit_synthetic (not by decision).
#[test]
fn repeated_decision_without_commit_returns_same_decision_until_committed() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), cam);
    assert_synthetic(state.begin_frame(cfg, ready_ctx(cam)));
    assert_synthetic(state.begin_frame(cfg, ready_ctx(cam)));
    state.commit_synthetic();
    assert_real_reason(
        state.begin_frame(cfg, ready_ctx(cam)),
        RealReason::LimitReached,
    );
}

#[test]
fn deterministic_same_inputs_same_decisions() {
    fn run() -> Vec<String> {
        let cfg = enabled_config();
        let mut state = TfrState::new();
        state.on_config_changed(cfg);
        let cams = [
            camera([0.0, 64.0, 0.0], 0.0, 0.0),
            camera([0.2, 64.0, 0.0], 1.0, 0.0),
            camera([20.0, 64.0, 0.0], 0.0, 0.0),
            camera([0.0, 64.0, 0.0], 30.0, 0.0),
        ];
        let mut out = Vec::new();
        for (i, cam) in cams.into_iter().enumerate() {
            let mut c = base_ctx(cam);
            c.world = world(if i == 2 { 2 } else { 1 });
            c.history_ready = i != 0;
            let d = state.begin_frame(cfg, c);
            out.push(format!("{d:?}"));
            match d {
                Decision::Real { .. } => commit_successful_real(&mut state, c.world, cam),
                Decision::Synthetic => state.commit_synthetic(),
            }
        }
        out
    }
    assert_eq!(run(), run());
}

// Fallback/error recovery: no valid synthetic-eligible state until successful real commit.
#[test]
fn fallback_real_invalidates_synthetic_eligibility() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), cam);
    state.note_fallback_real();
    assert!(!state.has_valid_history());
    assert!(!matches!(
        state.begin_frame(cfg, ready_ctx(cam)),
        Decision::Synthetic
    ));
}

#[test]
fn fallback_then_fresh_real_can_reenable_synthetic() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    let w = world(1);
    commit_successful_real(&mut state, w, cam);
    state.note_fallback_real();
    assert!(!matches!(
        state.begin_frame(cfg, ready_ctx(cam)),
        Decision::Synthetic
    ));
    commit_successful_real(&mut state, w, cam);
    assert_synthetic(state.begin_frame(cfg, ready_ctx(cam)));
}

#[test]
fn no_synthetic_without_fresh_successful_real_after_invalidate() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), cam);
    state.invalidate("test");
    let reason = assert_real(state.begin_frame(cfg, base_ctx(cam)));
    assert!(matches!(
        reason,
        RealReason::NoHistory | RealReason::Invalidated
    ));
}

#[test]
fn history_ready_false_invalidates_stale_history() {
    let cfg = enabled_config();
    let mut state = TfrState::new();
    state.on_config_changed(cfg);
    let cam = camera([0.0, 64.0, 0.0], 0.0, 0.0);
    commit_successful_real(&mut state, world(1), cam);
    let mut c = ready_ctx(cam);
    c.history_ready = false;
    assert_real_reason(state.begin_frame(cfg, c), RealReason::NoHistory);
    assert!(!state.has_valid_history());
}

// Documented helper (implementation-aligned): world_post-compatible clamp 1..8192.
#[test]
fn scene_size_world_post_compatible_clamps_to_device_like_range() {
    assert_eq!(
        scene_size(
            FrameSize {
                width: 1920,
                height: 1080
            },
            10.0
        ),
        FrameSize {
            width: 8192,
            height: 8192
        }
    );
}

// Pitch is clamped to valid range; yaw is wrapped for shortest angular delta (CM-02 / math conventions).
#[test]
fn pitch_is_clamped_not_wrapped() {
    assert!((clamp_pitch_deg(95.0) - 90.0).abs() < 1e-6);
    assert!((clamp_pitch_deg(-95.0) - (-90.0)).abs() < 1e-6);
    assert!((wrap_degrees(190.0) - (-170.0)).abs() < 1e-6);
    let a = camera([0.0, 64.0, 0.0], 0.0, 95.0);
    let b = camera([0.0, 64.0, 0.0], 0.0, -95.0);
    // clamped to +90 and -90 => 180 deg apart, not wrapped to small delta
    assert!(
        angle_metric_deg(&a, &b) >= 179.9,
        "{}",
        angle_metric_deg(&a, &b)
    );
}

// Unified meaningful-motion epsilons (documented, fixed, below cut ranges).
#[test]
fn meaningful_motion_epsilons_are_fixed() {
    assert!((MOTION_POS_EPS - 0.001).abs() < 1e-12);
    assert!((MOTION_POS_EPS2 - 0.000001).abs() < 1e-18);
    assert!((MOTION_ANGLE_EPS_DEG - 0.01).abs() < 1e-9);
}

#[test]
fn distance_sq_is_world_space_squared_displacement() {
    let a = camera([1.0, 2.0, 3.0], 0.0, 0.0);
    let b = camera([4.0, 6.0, 3.0], 0.0, 0.0); // dx=3,dy=4,dz=0
    assert!((distance_sq(&a, &b) - 25.0).abs() < 1e-12);
}

// --- GPU/math-helper/integration coverage required by spec (not expressible with current CPU core API) ---
// - Rotation-only VP convention and sky/far reprojection without camera translation (M-04, MF-02, reversed-Z far~0).
// - Motion sign/convention: history lookup uses `uv - motion` (or equivalent) per final shader/API contract (MF-03, SF-02).
// - Unified w/depth/screen tolerances and NDC/depth mapping in Horizon reversed-Z (M-10, MF-04).
// - HDR alpha/mark preservation for real blit and synthetic output (HDR-01/02/03).
// These must be covered by Agent #2 math/passes/GPU tests or integration tests once APIs exist; blocking for ready when implemented.
