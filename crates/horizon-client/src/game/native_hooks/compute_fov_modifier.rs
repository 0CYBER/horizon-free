use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Player FOV scale, after vanilla effects and before zoom and easing.
    /// Invalid values or values outside 0.01..=10 revert to base. API 362.
    pub(in crate::game) fn native_compute_fov_modifier(
        &self,
        player: &mod_api::client::camera::FovPlayer,
        base: f32,
        modifier: &mut f32,
    ) {
        self.fire_compute_fov_modifier(player, base, modifier);
    }
}
