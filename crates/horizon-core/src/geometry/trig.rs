//! Vanilla's quantized float sine table with Java double-to-long indexing.
use std::sync::OnceLock;

pub const DEG_TO_RAD: f32 = f32::from_bits(0x3c8e_fa35);
pub const RAD_TO_DEG: f32 = f32::from_bits(0x4265_2ee0);
const SCALE: f64 = 10430.378350470453;
static SINE: OnceLock<Box<[f32]>> = OnceLock::new();

/// Generated from the 26.3 table formula, with no bundled vanilla artifact.
/// Differential validation checks all 65,536 entries on the tested platform.
pub fn sine_table() -> &'static [f32] {
    SINE.get_or_init(|| {
        (0..65536)
            .map(|i| (i as f64 / SCALE).sin() as f32)
            .collect::<Vec<_>>()
            .into_boxed_slice()
    })
}
pub fn sin(angle: f64) -> f32 {
    sine_table()[((angle * SCALE) as i64 & 65535) as usize]
}
pub fn cos(angle: f64) -> f32 {
    sine_table()[((angle * SCALE + 16384.0) as i64 & 65535) as usize]
}

/// `Mth.sin` / `Mth.cos` on a float, as the models call them.
pub trait Mth {
    fn msin(self) -> f32;
    fn mcos(self) -> f32;
}
impl Mth for f32 {
    fn msin(self) -> f32 {
        sin(f64::from(self))
    }
    fn mcos(self) -> f32 {
        cos(f64::from(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_narrowing_and_quadrants() {
        assert_eq!(sin(0.0).to_bits(), 0.0f32.to_bits());
        assert_eq!(sin(f64::NAN), sin(0.0));
        assert_eq!(sin(f64::INFINITY), sine_table()[65535]);
        assert_eq!(sin(f64::NEG_INFINITY), sine_table()[0]);
        assert_eq!(cos(0.0), 1.0);
        assert_eq!(sin(std::f64::consts::FRAC_PI_2), 1.0);
    }
}
