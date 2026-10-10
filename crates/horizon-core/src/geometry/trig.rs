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

/// `Mth.atan2`: vanilla's table-driven arctangent.
pub fn atan2(mut y: f64, mut x: f64) -> f64 {
    static TABLES: OnceLock<(Vec<f64>, Vec<f64>)> = OnceLock::new();
    let (asin, cos) = TABLES.get_or_init(|| {
        (0..257)
            .map(|i| {
                let a = (f64::from(i) / 256.0).asin();
                (a, a.cos())
            })
            .unzip()
    });
    let frac_bias = f64::from_bits(4_805_340_802_404_319_232);
    let d2 = x * x + y * y;
    if d2.is_nan() {
        return f64::NAN;
    }
    let negative_y = y < 0.0;
    if negative_y {
        y = -y;
    }
    let negative_x = x < 0.0;
    if negative_x {
        x = -x;
    }
    let steep = y > x;
    if steep {
        std::mem::swap(&mut x, &mut y);
    }
    // Mth.fastInvSqrt
    let half = 0.5 * d2;
    let mut r = f64::from_bits((6_910_469_410_427_058_090i64 - (d2.to_bits() as i64 >> 1)) as u64);
    r *= 1.5 - half * r * r;
    x *= r;
    y *= r;
    let yp = frac_bias + y;
    let index = yp.to_bits() as i32 as usize;
    let s_phi = yp - frac_bias;
    let sd = y * cos[index] - x * s_phi;
    let mut theta = asin[index] + (6.0 + sd * sd) * sd * 0.166_666_666_666_666_66;
    if steep {
        theta = std::f64::consts::FRAC_PI_2 - theta;
    }
    if negative_x {
        theta = std::f64::consts::PI - theta;
    }
    if negative_y {
        theta = -theta;
    }
    theta
}
/// `(float)(Mth.atan2(y, x) * 180.0F / (float)Math.PI)`: the angle in degrees
/// as the AI controls compute it.
pub fn atan2_degrees(y: f64, x: f64) -> f32 {
    (atan2(y, x) * 180.0 / f64::from(std::f32::consts::PI)) as f32
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
