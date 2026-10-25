//! No HD haptics on iOS: they play through the pad's own four-channel audio device over USB,
//! which needs cpal, and cpal is left out there with hidapi (see Cargo.toml).

/// The pad's audio output, playing the queued samples (left, right actuator).
pub struct Haptics;

impl Haptics {
    /// The DualSense's audio device, if one is there with its four channels (USB). A phone
    /// has none.
    pub fn open() -> Option<Haptics> {
        None
    }

    /// Play a vibration (heavy and light levels, 0..1, each for a while), replacing the one
    /// playing. Nothing plays on iOS.
    pub fn play(&self, _steps: &[(f32, f32, std::time::Duration)]) {}

    /// Stop at once. Nothing plays on iOS.
    pub fn stop(&self) {}
}
