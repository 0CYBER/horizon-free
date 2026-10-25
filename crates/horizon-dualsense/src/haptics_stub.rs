//! No HD haptics on iOS: the trick plays vibrations through the pad's own four-channel audio
//! device over USB, and a phone has neither that device nor the audio stack the real module
//! needs. The motors' rumble through [`crate::Link`] covers vibration instead.

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
