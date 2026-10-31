//! Frame generation: FSR 3.1's frame interpolation will show a frame halfway between each
//! two real ones. So far this holds its optical flow; the interpolation and the setting
//! that runs them come next.

// Unused until the interpolation and the renderer use it.
#[allow(dead_code)]
mod optical_flow;
