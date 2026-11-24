//! TFR configuration (spec §2, `TFR_DESCRIPTION.md`).

/// Temporal Frame Reprojection parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TfrConfig {
    /// Enables TFR (fixed alternation: one successful real, then at most one successful synthetic).
    pub enabled: bool,
    /// Reprojection strength in `[0.0, 1.0]`.
    pub reprojection_strength: f32,
    /// History stability in `[0.0, 0.35]`.
    pub history_stability: f32,
    /// Distance cut threshold in world units `[0.25, 16.0]`.
    pub cut_distance: f32,
    /// Angle cut threshold in degrees `[5.0, 90.0]`.
    pub cut_angle_deg: f32,
}

impl Default for TfrConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            reprojection_strength: 1.0,
            history_stability: 0.06,
            cut_distance: 9.25,
            cut_angle_deg: 14.0,
        }
    }
}

impl TfrConfig {
    /// Returns a copy clamped to the spec ranges. Non-finite values fall back to defaults.
    #[must_use]
    pub fn sanitized(mut self) -> Self {
        self.reprojection_strength = if self.reprojection_strength.is_finite() {
            self.reprojection_strength.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.history_stability = if self.history_stability.is_finite() {
            self.history_stability.clamp(0.0, 0.35)
        } else {
            0.06
        };
        self.cut_distance = if self.cut_distance.is_finite() {
            self.cut_distance.clamp(0.25, 16.0)
        } else {
            9.25
        };
        self.cut_angle_deg = if self.cut_angle_deg.is_finite() {
            self.cut_angle_deg.clamp(5.0, 90.0)
        } else {
            14.0
        };
        self
    }
}
