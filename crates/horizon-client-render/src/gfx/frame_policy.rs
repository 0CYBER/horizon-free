//! The frame policy: which frame a renderer draws, what the overlays draw on and whether
//! Horizon's tone curve runs (tests in `frame_policy_tests.rs`).

use super::LINEAR_FRAME;

/// What a renderer's frame is drawn in: `LINEAR_FRAME` where the device draws it, for a 0.2
/// pack, or for the layered HDR frame where the surface offers HDR output (`linear`:
/// `(a 0.2 pack picked, the layered HDR frame, the device draws it)`). Else the swapchain's
/// 8-bit format, as without HDR: a `shaders/` pack's SDR picture too.
pub(super) fn frame_format(
    hdr_offered: bool,
    swapchain: wgpu::TextureFormat,
    linear: (bool, bool, bool),
) -> wgpu::TextureFormat {
    if linear.2 && (linear.0 || linear.1 && hdr_offered) {
        LINEAR_FRAME
    } else {
        swapchain.remove_srgb_suffix()
    }
}

/// The frame's format, and whether it is the layered HDR frame (HUD coverage): for HDR on
/// with no pack or a 0.2 one (`hdr_float`), where the device keeps the HUD on a layer of its
/// own. HDR is on only where it does (`output::OutputCapabilities::hdr_frame`). `device`:
/// the surface offers HDR output, the device draws the float frame, and it keeps that layer.
pub(super) fn frame_configuration(
    device: (bool, bool, bool),
    swapchain: wgpu::TextureFormat,
    linear_pack: bool,
    hdr_float: bool,
) -> (wgpu::TextureFormat, bool) {
    let (hdr_offered, linear_ok, coverage_ok) = device;
    let layered = hdr_float && hdr_offered && linear_ok && coverage_ok;
    (
        frame_format(hdr_offered, swapchain, (linear_pack, layered, linear_ok)),
        layered,
    )
}

/// What the hand, the camera overlays and the HUD draw on: the frame, except for a 0.2 pack's
/// float frame without the HUD's layer, which is SDR: the pack's picture finishes on the
/// swapchain's 8-bit format first, so they clip and blend per draw as on vanilla's frame.
pub(super) fn overlay_format(
    frame: wgpu::TextureFormat,
    layered: bool,
    swapchain: wgpu::TextureFormat,
) -> wgpu::TextureFormat {
    if frame == LINEAR_FRAME && !layered {
        swapchain.remove_srgb_suffix()
    } else {
        frame
    }
}

/// What the hand's pass does with the depth it drew: kept on HDR's layered frame, where a
/// resource pack's post effect may read it after the hand (`Overlay::world_depth`).
pub(super) fn hand_depth_store(layered: bool) -> wgpu::StoreOp {
    if layered {
        wgpu::StoreOp::Store
    } else {
        wgpu::StoreOp::Discard
    }
}

/// Whether Horizon's tone curve (`Hdr::tonemap`) runs on this frame: on the layered frame's
/// world, unless a 0.2 pack's `[tonemap]` already mapped it. One tone mapping, never two.
pub(super) fn horizon_tone_curve(layered: bool, world: bool, pack_tonemap: bool) -> bool {
    layered && world && !pack_tonemap
}
