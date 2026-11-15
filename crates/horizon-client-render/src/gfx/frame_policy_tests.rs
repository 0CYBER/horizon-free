//! The frame policy: which frame a renderer draws (`frame_configuration`), and what follows
//! from the frame chosen.

#[test]
fn float_frames_never_schedule_light_mark_passes() {
    use super::{LINEAR_FRAME, WIDE_FRAME, light_marks};
    use wgpu::TextureFormat::{Bgra8Unorm, Rgb10a2Unorm, Rgba8Unorm};
    for hdr in [false, true] {
        assert!(!light_marks(LINEAR_FRAME, hdr));
        for format in [WIDE_FRAME, Bgra8Unorm, Rgba8Unorm, Rgb10a2Unorm] {
            assert_eq!(light_marks(format, hdr), hdr);
        }
    }
}

#[test]
fn frame_policy_keeps_compatibility_frames_and_requires_layering_for_float_hdr() {
    use super::{LINEAR_FRAME, WIDE_FRAME, frame_configuration};
    use wgpu::TextureFormat::{Bgra8Unorm, Bgra8UnormSrgb};

    for bits in 0..64 {
        let hdr = bits & 1 != 0;
        let offered = bits & 2 != 0;
        let wide_ok = bits & 4 != 0;
        let float_ok = bits & 8 != 0;
        let coverage_ok = bits & 16 != 0;
        let smooth = bits & 32 != 0;
        for pack in ["none", "0.1", "0.2", "shaders"] {
            let float_hdr = matches!(pack, "none" | "0.2");
            let expected = if float_ok && float_hdr && hdr && offered && coverage_ok {
                (LINEAR_FRAME, true)
            } else if float_ok && pack == "0.2" {
                (LINEAR_FRAME, false)
            } else if hdr && smooth && offered && wide_ok {
                (WIDE_FRAME, false)
            } else {
                (Bgra8Unorm, false)
            };
            assert_eq!(
                frame_configuration(
                    hdr && smooth,
                    (offered, wide_ok, float_ok, coverage_ok),
                    Bgra8UnormSrgb,
                    pack == "0.2",
                    hdr && float_hdr,
                ),
                expected,
                "bits {bits}, pack {pack}"
            );
        }
    }
}

/// A device that can't keep the HUD on a layer of its own, or can't draw the float frame,
/// gets no HDR, under no pack and no mode: no compatibility frame stands in for HDR's.
#[test]
fn hdr_stays_off_for_each_missing_requirement_of_its_frame() {
    use super::output::{FrameRequest, tests};
    use super::{LINEAR_FRAME, hdr::Output, hud_coverage};
    use crate::graphics::{Graphics, HdrMode};
    use wgpu::{DownlevelFlags as D, TextureFormat as F, TextureFormatFeatureFlags as Fl};
    use wgpu::{TextureFormatFeatures, TextureUsages as U};

    let usages = U::RENDER_ATTACHMENT | U::TEXTURE_BINDING;
    let coverage = |attachments, flags, allowed_usages, format_flags| {
        let format = TextureFormatFeatures {
            allowed_usages,
            flags: format_flags,
        };
        hud_coverage::supported(attachments, flags, format)
    };
    let blend = D::INDEPENDENT_BLEND;
    // (the float frame, the HUD's layer): no float frame, then each thing the layer needs.
    let devices = [
        (false, true),
        (true, coverage(1, blend, usages, Fl::BLENDABLE)),
        (true, coverage(2, D::empty(), usages, Fl::BLENDABLE)),
        (true, coverage(2, blend, U::TEXTURE_BINDING, Fl::BLENDABLE)),
        (
            true,
            coverage(2, blend, U::RENDER_ATTACHMENT, Fl::BLENDABLE),
        ),
        (true, coverage(2, blend, usages, Fl::empty())),
    ];
    let no_pack = FrameRequest::default();
    let pack_0_1 = FrameRequest {
        compatibility: true,
        ..no_pack
    };
    let pack_0_2 = FrameRequest {
        linear_pack: true,
        ..no_pack
    };
    for (float_frame, hud_layer) in devices {
        for mode in [HdrMode::On, HdrMode::Auto] {
            let settings = Graphics {
                hdr: mode,
                ..Graphics::default()
            };
            let (mut caps, reading) = tests::window(Some(Output::Pq), Some(true));
            (caps.float_frame, caps.hud_layer) = (float_frame, hud_layer);
            let case = format!("{mode:?}, float {float_frame}, layer {hud_layer}");
            for request in [no_pack, pack_0_1] {
                let out = tests::snapshot(&settings, request, caps, reading);
                assert_eq!(out.frame.format, F::Bgra8Unorm, "{case}");
                assert!(!out.frame.hud_layer && !out.hdr_on(), "{case}");
                assert!(!out.requests_hdr(mode) && out.hdr_offered(), "{case}");
            }
            // A 0.2 pack keeps its float frame where the device draws one, without the layer: SDR.
            let out = tests::snapshot(&settings, pack_0_2, caps, reading);
            let format = if float_frame {
                LINEAR_FRAME
            } else {
                F::Bgra8Unorm
            };
            assert_eq!(out.frame.format, format, "{case}");
            assert_eq!(out.frame.overlay_format, F::Bgra8Unorm, "{case}");
            assert!(!out.frame.hud_layer && !out.hdr_on(), "{case}");
        }
    }
}

#[test]
fn a_0_2_pack_without_the_hud_layer_finishes_on_the_swapchains_8_bits() {
    use super::{LINEAR_FRAME, WIDE_FRAME, overlay_format};
    use wgpu::TextureFormat::{Bgra8Unorm, Bgra8UnormSrgb, Rgba8Unorm};
    assert_eq!(
        overlay_format(LINEAR_FRAME, false, Bgra8UnormSrgb),
        Bgra8Unorm
    );
    assert_eq!(overlay_format(LINEAR_FRAME, false, Rgba8Unorm), Rgba8Unorm);
    // Everything else draws its overlays on its frame: the layered frame's light, the
    // 16-bit frame, the 8-bit frame.
    for (frame, layered) in [
        (LINEAR_FRAME, true),
        (WIDE_FRAME, false),
        (Bgra8Unorm, false),
    ] {
        assert_eq!(overlay_format(frame, layered, Bgra8UnormSrgb), frame);
    }
}

#[test]
fn one_tone_curve_runs_on_the_layered_world() {
    use super::horizon_tone_curve;
    // No pack, or a 0.2 pack without `[tonemap]`: Horizon's curve.
    assert!(horizon_tone_curve(true, true, false));
    // A pack's `[tonemap]` mapped it already; never both.
    assert!(!horizon_tone_curve(true, true, true));
    // No world (the title screen), or no layered frame (SDR, the compatibility frames).
    assert!(!horizon_tone_curve(true, false, false));
    assert!(!horizon_tone_curve(false, true, false));
}

#[test]
fn only_a_no_pack_layered_world_applies_hdr_gain() {
    use super::no_pack_hdr_gain;
    assert!(no_pack_hdr_gain(true, false, true));
    assert!(!no_pack_hdr_gain(false, false, true));
    assert!(!no_pack_hdr_gain(true, true, true));
    assert!(!no_pack_hdr_gain(true, false, false));
}
