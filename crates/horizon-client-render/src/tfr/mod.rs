//! Temporal Frame Reprojection (TFR): alternate real and synthetic world frames.
//!
//! Clean-room implementation. See `docs/tfr-design.md`.

pub mod config;
pub mod core;
pub mod math;
pub mod passes;

pub use config::TfrConfig;
